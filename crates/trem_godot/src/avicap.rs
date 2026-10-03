//! The `video` capture of the Godot client (dec038): cl_main.c
//! `CL_Video_f` / `CL_StopVideo_f` / `CL_Frame`'s fixed frame time,
//! cl_avi.c through the kernel's `AviFile`, the renderer's
//! `RE_TakeVideoFrame` / `RB_TakeVideoFrameCmd` (the viewport image as
//! motion JPEG at r_aviMotionJpegQuality, or raw BGR) and snd_mix.c
//! `S_TransferPaintBuffer`'s `CL_WriteAVIAudioFrame` (the Master bus mix
//! through an AudioEffectCapture, 16-bit stereo at the output mix rate).
//!
//! The mixer runs in real time in Godot, so each video frame takes exactly
//! the samples `S_GetSoundtime` gives it (rate / cl_aviFrameRate) from the
//! captured mix: missing samples are silence, a surplus (the game running
//! slower than the video's frame rate) is dropped, which keeps the audio
//! stream as long as the video.

use godot::classes::image::Format;
use godot::classes::{AudioEffectCapture, AudioServer, Viewport};
use godot::prelude::*;
use std::path::Path;
use trem_sim::kernel::avi::{raw_frame_from_rgb, AviFile, DmaFormat};

/// An open video and its audio tap.
pub struct VideoCapture {
    pub avi: AviFile,
    fx: Option<Gd<AudioEffectCapture>>,
    fx_index: i32,
}

/// The client cvars of cl_avi.c / the renderer (name, default).
pub const AVI_CVARS: &[(&str, &str)] = &[("cl_aviFrameRate", "25"), ("cl_aviMotionJpeg", "1"), ("r_aviMotionJpegQuality", "90"), ("cl_forceavidemo", "0")];

impl VideoCapture {
    /// `CL_OpenAVIForWriting` at the viewport's size with the Master bus as
    /// the audio stream (`dma`: the output mix rate, 2 channels, 16 bits).
    pub fn open(path: &Path, frame_rate: i32, motion_jpeg: bool, viewport: &Gd<Viewport>) -> Result<VideoCapture, String> {
        let size = viewport.get_visible_rect().size;
        let mut a = AudioServer::singleton();
        let rate = a.get_mix_rate() as i32;
        let avi = AviFile::open(path, frame_rate, size.x as i32, size.y as i32, motion_jpeg, Some(DmaFormat { speed: rate, channels: 2, samplebits: 16 }))?;
        let mut fx = AudioEffectCapture::new_gd();
        fx.set_buffer_length(2.0);
        let fx_index = a.get_bus_effect_count(0);
        a.add_bus_effect(0, &fx);
        Ok(VideoCapture { avi, fx: Some(fx), fx_index })
    }

    /// `CL_TakeVideoFrame` + this frame's `CL_WriteAVIAudioFrame`.
    pub fn take_frame(&mut self, viewport: &Gd<Viewport>, quality: i32) -> Result<(), String> {
        let (w, h) = (self.avi.width, self.avi.height);
        if let Some(mut img) = viewport.get_texture().and_then(|t| t.get_image()) {
            if img.get_width() != w || img.get_height() != h {
                img.resize(w, h);
            }
            if self.avi.motion_jpeg {
                // RE_SaveJPGToBuffer (top row first)
                let jpg = img.save_jpg_to_buffer_ex().quality(quality.clamp(1, 100) as f32 / 100.0).done();
                self.avi.write_video_frame(jpg.as_slice())?;
            } else {
                img.convert(Format::RGB8);
                let rgb = img.get_data();
                self.avi.write_video_frame(&raw_frame_from_rgb(rgb.as_slice(), w as usize, h as usize))?;
            }
        }
        // the sound time of one video frame
        let n = self.avi.sound_frame_samples().max(0) as usize;
        let mut pcm = Vec::with_capacity(n * 4);
        if let Some(fx) = self.fx.as_mut() {
            let avail = fx.get_frames_available().max(0) as usize;
            let take = avail.min(n);
            if take > 0 {
                for f in fx.get_buffer(take as i32).as_slice() {
                    for v in [f.x, f.y] {
                        pcm.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
                    }
                }
            }
            if avail > n {
                // the mix ran ahead of the video's clock
                fx.get_buffer((avail - n) as i32);
            }
        }
        pcm.resize(n * 4, 0);
        self.avi.write_audio_frame(&pcm)
    }

    /// `CL_CloseAVI`.
    pub fn close(mut self) -> Result<String, String> {
        if let Some(_fx) = self.fx.take() {
            let mut a = AudioServer::singleton();
            if self.fx_index < a.get_bus_effect_count(0) {
                a.remove_bus_effect(0, self.fx_index);
            }
        }
        self.avi.close()
    }
}
