//! Kernel `avi` — client/cl_avi.c, the `video` command's AVI writer:
//! `CL_WriteAVIHeader` (RIFF/AVI with an MJPG or BI_RGB video stream and an
//! optional PCM audio stream), `CL_OpenAVIForWriting`, `CL_CheckFileSize`,
//! `CL_WriteAVIVideoFrame`, `CL_WriteAVIAudioFrame`, `CL_CloseAVI` (the
//! idx1 index appended, the real header written over the placeholder),
//! `CL_VideoRecording`; cl_main.c `CL_Video_f`'s file name scan and
//! `CL_Frame`'s fixed frame time while recording, snd_dma.c
//! `S_GetSoundtime`'s fixed sound time, and renderergl1 tr_init.c
//! `RB_TakeVideoFrameCmd`'s raw BGR frame layout.
//!
//! The index is kept in memory instead of the C's temporary
//! "<name>.index.dat" file; the bytes written are the same.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// `MAX_RIFF_CHUNKS`.
const MAX_RIFF_CHUNKS: usize = 16;
/// `AVI_LINE_PADDING`: raw avi files have pixel lines start on 4-byte boundaries.
pub const AVI_LINE_PADDING: usize = 4;
/// `WAV_FORMAT_PCM`.
pub const WAV_FORMAT_PCM: i32 = 1;
/// `PCM_BUFFER_SIZE`.
const PCM_BUFFER_SIZE: usize = 44100;

/// `PAD( x, y )`.
pub fn pad(x: usize, y: usize) -> usize {
    (x + y - 1) & !(y - 1)
}

/// `PADLEN( x, y )`.
pub fn padlen(x: usize, y: usize) -> usize {
    pad(x, y) - x
}

/// `audioFormat_t`.
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioFormat {
    pub rate: i32,
    pub format: i32,
    pub channels: i32,
    pub bits: i32,
    pub sample_size: i32,
    pub total_bytes: i32,
}

/// The header buffer with `START_CHUNK` / `END_CHUNK`'s chunk stack.
struct Buf {
    b: Vec<u8>,
    stack: Vec<usize>,
}

impl Buf {
    fn new() -> Buf {
        Buf { b: Vec::new(), stack: Vec::new() }
    }
    /// `WRITE_STRING`
    fn s(&mut self, s: &str) {
        self.b.extend_from_slice(s.as_bytes());
    }
    /// `WRITE_4BYTES`
    fn l(&mut self, x: i32) {
        self.b.extend_from_slice(&x.to_le_bytes());
    }
    /// `WRITE_2BYTES`
    fn w(&mut self, x: i32) {
        self.b.extend_from_slice(&(x as u16).to_le_bytes());
    }
    /// `START_CHUNK`
    fn start(&mut self, s: &str) {
        assert!(self.stack.len() < MAX_RIFF_CHUNKS, "ERROR: Top of chunkstack breached");
        self.stack.push(self.b.len());
        self.s(s);
        self.l(0);
    }
    /// `END_CHUNK`
    fn end(&mut self) {
        let end = self.b.len();
        let at = self.stack.pop().expect("ERROR: Bottom of chunkstack breached") + 4;
        let size = (end - at - 4) as i32;
        self.b[at..at + 4].copy_from_slice(&size.to_le_bytes());
        self.b.resize(pad(end, 2), 0);
    }
}

/// `aviFileData_t`: an open video.
pub struct AviFile {
    f: File,
    pub file_name: PathBuf,
    pub file_size: i64,
    pub movi_offset: usize,
    pub movi_size: i32,
    /// the idx1 entries (the C's temporary index file)
    index: Vec<u8>,
    pub num_indices: i32,
    pub frame_rate: i32,
    pub frame_period: i32,
    pub width: i32,
    pub height: i32,
    pub num_video_frames: i32,
    pub max_record_size: i32,
    pub motion_jpeg: bool,
    pub audio: bool,
    pub a: AudioFormat,
    pub num_audio_frames: i32,
    /// `pcmCaptureBuffer` / `bytesInBuffer`
    pcm: Vec<u8>,
    /// `clc.aviVideoFrameRemainder`
    pub video_frame_remainder: f32,
    /// `clc.aviSoundFrameRemainder`
    pub sound_frame_remainder: f32,
    /// Com_Printf text for the host
    pub print_out: String,
}

/// What the sound system offers for the audio stream (`dma`): None
/// without sound (s_initsound 0).
#[derive(Clone, Copy, Debug)]
pub struct DmaFormat {
    pub speed: i32,
    pub channels: i32,
    pub samplebits: i32,
}

impl AviFile {
    /// `CL_WriteAVIHeader`: everything up to the open "LIST" "movi" chunk;
    /// the movi offset.
    fn header(&self) -> (Buf, usize) {
        let mut b = Buf::new();
        b.start("RIFF");
        b.s("AVI ");
        b.start("LIST");
        b.s("hdrl");
        b.s("avih");
        b.l(56); //"avih" "chunk" size
        b.l(self.frame_period); //dwMicroSecPerFrame
        b.l(self.max_record_size.wrapping_mul(self.frame_rate)); //dwMaxBytesPerSec
        b.l(0); //dwReserved1
        b.l(0x110); //dwFlags bits HAS_INDEX and IS_INTERLEAVED
        b.l(self.num_video_frames); //dwTotalFrames
        b.l(0); //dwInitialFrame
        b.l(if self.audio { 2 } else { 1 }); //dwStreams
        b.l(self.max_record_size); //dwSuggestedBufferSize
        b.l(self.width); //dwWidth
        b.l(self.height); //dwHeight
        for _ in 0..4 {
            b.l(0); //dwReserved
        }
        b.start("LIST");
        b.s("strl");
        b.s("strh");
        b.l(56); //"strh" "chunk" size
        b.s("vids");
        if self.motion_jpeg {
            b.s("MJPG");
        } else {
            b.l(0); // BI_RGB
        }
        b.l(0); //dwFlags
        b.l(0); //dwPriority
        b.l(0); //dwInitialFrame
        b.l(1); //dwTimescale
        b.l(self.frame_rate); //dwDataRate
        b.l(0); //dwStartTime
        b.l(self.num_video_frames); //dwDataLength
        b.l(self.max_record_size); //dwSuggestedBufferSize
        b.l(-1); //dwQuality
        b.l(0); //dwSampleSize
        b.w(0); //rcFrame
        b.w(0);
        b.w(self.width);
        b.w(self.height);
        b.s("strf");
        b.l(40); //"strf" "chunk" size
        b.l(40); //biSize
        b.l(self.width); //biWidth
        b.l(self.height); //biHeight
        b.w(1); //biPlanes
        b.w(24); //biBitCount
        if self.motion_jpeg {
            b.s("MJPG"); //biCompression
            b.l(self.width * self.height); //biSizeImage
        } else {
            b.l(0); // BI_RGB
            b.l(self.width * self.height * 3); //biSizeImage
        }
        b.l(0); //biXPelsPetMeter
        b.l(0); //biYPelsPetMeter
        b.l(0); //biClrUsed
        b.l(0); //biClrImportant
        b.end();
        if self.audio {
            b.start("LIST");
            b.s("strl");
            b.s("strh");
            b.l(56); //"strh" "chunk" size
            b.s("auds");
            b.l(0); //FCC
            b.l(0); //dwFlags
            b.l(0); //dwPriority
            b.l(0); //dwInitialFrame
            b.l(self.a.sample_size); //dwTimescale
            b.l(self.a.sample_size * self.a.rate); //dwDataRate
            b.l(0); //dwStartTime
            b.l(self.a.total_bytes / self.a.sample_size); //dwDataLength
            b.l(0); //dwSuggestedBufferSize
            b.l(-1); //dwQuality
            b.l(self.a.sample_size); //dwSampleSize
            for _ in 0..4 {
                b.w(0); //rcFrame
            }
            b.s("strf");
            b.l(18); //"strf" "chunk" size
            b.w(self.a.format); //wFormatTag
            b.w(self.a.channels); //nChannels
            b.l(self.a.rate); //nSamplesPerSec
            b.l(self.a.sample_size * self.a.rate); //nAvgBytesPerSec
            b.w(self.a.sample_size); //nBlockAlign
            b.w(self.a.bits); //wBitsPerSample
            b.w(0); //cbSize
            b.end();
        }
        b.end();
        let movi_offset = b.b.len();
        b.start("LIST");
        b.s("movi");
        (b, movi_offset)
    }

    /// `CL_OpenAVIForWriting`: creates an AVI file and gets it into a state
    /// where writing the actual data can begin. `frame_rate` is
    /// cl_aviFrameRate, the size the video's (`cls.glconfig.vidWidth/Height`).
    pub fn open(file_name: &Path, frame_rate: i32, width: i32, height: i32, motion_jpeg: bool, dma: Option<DmaFormat>) -> Result<AviFile, String> {
        // Don't start if a framerate has not been chosen
        if frame_rate <= 0 {
            return Err("^1cl_aviFrameRate must be >= 1\n".into());
        }
        if let Some(d) = file_name.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let f = File::create(file_name).map_err(|e| format!("Couldn't open {} for writing: {e}\n", file_name.display()))?;
        let mut afd = AviFile {
            f,
            file_name: file_name.to_path_buf(),
            file_size: 0,
            movi_offset: 0,
            movi_size: 0,
            index: Vec::new(),
            num_indices: 0,
            frame_rate,
            frame_period: (1000000.0f32 / frame_rate as f32) as i32,
            width,
            height,
            num_video_frames: 0,
            max_record_size: 0,
            motion_jpeg,
            audio: false,
            a: AudioFormat::default(),
            num_audio_frames: 0,
            pcm: Vec::new(),
            video_frame_remainder: 0.0,
            sound_frame_remainder: 0.0,
            print_out: String::new(),
        };
        match dma {
            None => afd.audio = false,
            Some(d) => {
                afd.a = AudioFormat { rate: d.speed, format: WAV_FORMAT_PCM, channels: d.channels, bits: d.samplebits, sample_size: (d.samplebits / 8) * d.channels, total_bytes: 0 };
                if afd.a.rate % afd.frame_rate != 0 {
                    let mut suggest = afd.frame_rate;
                    while afd.a.rate % suggest != 0 && suggest >= 1 {
                        suggest -= 1;
                    }
                    afd.print_out.push_str(&format!("^3WARNING: cl_aviFrameRate is not a divisor of the audio rate, suggest {suggest}\n"));
                }
                if afd.a.bits != 16 || afd.a.channels != 2 {
                    afd.print_out.push_str(&format!("^3WARNING: Audio format of {} bit/{} channels not supported", afd.a.bits, afd.a.channels));
                    afd.audio = false;
                } else {
                    afd.audio = true;
                }
            }
        }
        // This doesn't write a real header, but allocates the
        // correct amount of space at the beginning of the file
        let (h, movi_offset) = afd.header();
        afd.movi_offset = movi_offset;
        afd.f.write_all(&h.b).map_err(|_| "Failed to write avi file".to_string())?;
        afd.file_size = h.b.len() as i64;
        // START_CHUNK( "idx1" ) of the index
        afd.movi_size = 4; // For the "movi"
        Ok(afd)
    }

    /// `SafeFS_Write` (`Com_Error( ERR_DROP, "Failed to write avi file" )`).
    fn safe_write(&mut self, b: &[u8]) -> Result<(), String> {
        self.f.write_all(b).map_err(|_| "Failed to write avi file".to_string())
    }

    /// `CL_CheckFileSize`: past 2 GB the current file is closed and
    /// "<name>_" opened; true then (the data is not written).
    pub fn check_file_size(&mut self, bytes_to_add: usize) -> Result<bool, String> {
        let new_file_size = self.file_size + bytes_to_add as i64 + self.num_indices as i64 * 16 + 4;
        // I assume all the operating systems we target can handle a 2Gb file
        if new_file_size > i32::MAX as i64 {
            // Close the current file...
            let name = format!("{}_", self.file_name.display());
            let dma = self.audio.then_some(DmaFormat { speed: self.a.rate, channels: self.a.channels, samplebits: self.a.bits });
            // ...And open a new one
            let new = AviFile::open(Path::new(&name), self.frame_rate, self.width, self.height, self.motion_jpeg, dma)?;
            let old = std::mem::replace(self, new);
            let out = old.close()?;
            self.print_out.insert_str(0, &out);
            return Ok(true);
        }
        Ok(false)
    }

    /// `CL_WriteAVIVideoFrame`: one "00dc" chunk and its index entry.
    pub fn write_video_frame(&mut self, image: &[u8]) -> Result<(), String> {
        let size = image.len();
        // Chunk header + contents + padding
        if self.check_file_size(8 + size + 2)? {
            return Ok(());
        }
        let chunk_offset = (self.file_size - self.movi_offset as i64 - 8) as i32;
        let chunk_size = 8 + size;
        let padding_size = padlen(size, 2);
        let mut h = Vec::with_capacity(8);
        h.extend_from_slice(b"00dc");
        h.extend_from_slice(&(size as i32).to_le_bytes());
        self.safe_write(&h)?;
        self.safe_write(image)?;
        self.safe_write(&[0u8; 4][..padding_size])?;
        self.file_size += (chunk_size + padding_size) as i64;
        self.num_video_frames += 1;
        self.movi_size += (chunk_size + padding_size) as i32;
        if size as i32 > self.max_record_size {
            self.max_record_size = size as i32;
        }
        // Index
        self.index.extend_from_slice(b"00dc"); //dwIdentifier
        self.index.extend_from_slice(&0x10i32.to_le_bytes()); //dwFlags (all frames are KeyFrames)
        self.index.extend_from_slice(&chunk_offset.to_le_bytes()); //dwOffset
        self.index.extend_from_slice(&(size as i32).to_le_bytes()); //dwLength
        self.num_indices += 1;
        Ok(())
    }

    /// `CL_WriteAVIAudioFrame`: PCM is buffered until a frame's worth is
    /// there, then written as one "01wb" chunk.
    pub fn write_audio_frame(&mut self, pcm: &[u8]) -> Result<(), String> {
        if !self.audio {
            return Ok(());
        }
        let mut size = pcm.len();
        // Chunk header + contents + padding
        if self.check_file_size(8 + self.pcm.len() + size + 2)? {
            return Ok(());
        }
        if self.pcm.len() + size > PCM_BUFFER_SIZE {
            self.print_out.push_str("^3WARNING: Audio capture buffer overflow -- truncating\n");
            size = PCM_BUFFER_SIZE - self.pcm.len();
        }
        self.pcm.extend_from_slice(&pcm[..size]);
        // Only write if we have a frame's worth of audio
        let frame_bytes = (self.a.rate as f32 / self.frame_rate as f32).ceil() as usize * self.a.sample_size as usize;
        if self.pcm.len() >= frame_bytes {
            let bytes = self.pcm.len();
            let chunk_offset = (self.file_size - self.movi_offset as i64 - 8) as i32;
            let chunk_size = 8 + bytes;
            let padding_size = padlen(bytes, 2);
            let mut h = Vec::with_capacity(8);
            h.extend_from_slice(b"01wb");
            h.extend_from_slice(&(bytes as i32).to_le_bytes());
            self.safe_write(&h)?;
            let data = std::mem::take(&mut self.pcm);
            self.safe_write(&data)?;
            self.safe_write(&[0u8; 4][..padding_size])?;
            self.file_size += (chunk_size + padding_size) as i64;
            self.num_audio_frames += 1;
            self.movi_size += (chunk_size + padding_size) as i32;
            self.a.total_bytes += bytes as i32;
            // Index
            self.index.extend_from_slice(b"01wb"); //dwIdentifier
            self.index.extend_from_slice(&0i32.to_le_bytes()); //dwFlags
            self.index.extend_from_slice(&chunk_offset.to_le_bytes()); //dwOffset
            self.index.extend_from_slice(&(bytes as i32).to_le_bytes()); //dwLength
            self.num_indices += 1;
        }
        Ok(())
    }

    /// `CL_CloseAVI`: closes the AVI file and writes an index chunk; the
    /// "Wrote %d:%d frames to %s" line.
    pub fn close(mut self) -> Result<String, String> {
        // Write index
        let index_size = self.num_indices * 16;
        let mut idx = Vec::with_capacity(8 + self.index.len());
        idx.extend_from_slice(b"idx1");
        idx.extend_from_slice(&index_size.to_le_bytes());
        idx.extend_from_slice(&std::mem::take(&mut self.index));
        self.safe_write(&idx)?;
        self.file_size += idx.len() as i64;
        // Write the real header
        self.f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut h = self.header().0.b;
        h[4..8].copy_from_slice(&((self.file_size - 8) as i32).to_le_bytes()); // "RIFF" size
        let m = self.movi_offset + 4; // Skip "LIST"
        h.truncate(m);
        h.extend_from_slice(&self.movi_size.to_le_bytes());
        self.safe_write(&h)?;
        self.f.flush().map_err(|e| e.to_string())?;
        let mut out = std::mem::take(&mut self.print_out);
        out.push_str(&format!("Wrote {}:{} frames to {}\n", self.num_video_frames, self.num_audio_frames, self.file_name.display()));
        Ok(out)
    }

    /// `CL_Frame`'s "if recording an avi, lock to a fixed fps": the frame
    /// time for this frame (cl_aviFrameRate * timescale, at most 1000 fps).
    pub fn video_frame_msec(&mut self, timescale: f32) -> i32 {
        let fps = (self.frame_rate as f32 * timescale).min(1000.0);
        let frame_duration = (1000.0 / fps).max(1.0) + self.video_frame_remainder;
        let msec = frame_duration as i32;
        self.video_frame_remainder = frame_duration - msec as f32;
        msec
    }

    /// `S_GetSoundtime` while recording: the sample frames the mixer
    /// advances by this video frame.
    pub fn sound_frame_samples(&mut self) -> i32 {
        let fps = (self.frame_rate as f32).min(1000.0);
        let frame_duration = (self.a.rate as f32 / fps).max(1.0) + self.sound_frame_remainder;
        let n = frame_duration as i32;
        self.sound_frame_remainder = frame_duration - n as f32;
        n
    }
}

/// `CL_Video_f`'s name: videos/<name>.avi, else the first free
/// videos/videoNNNN.avi (None when all 10000 are taken).
pub fn video_file_name(arg: Option<&str>, exists: impl Fn(&str) -> bool) -> Option<String> {
    if let Some(a) = arg {
        // explicit filename
        return Some(format!("videos/{a}.avi"));
    }
    // scan for a free filename
    (0..=9999).map(|i| format!("videos/video{i:04}.avi")).find(|f| !exists(f))
}

/// `RB_TakeVideoFrameCmd`'s raw path: top-down RGB rows (the viewport's
/// image) to the bottom-up BGR rows of a BI_RGB frame, each padded to 4
/// bytes (`glReadPixels` reads bottom-up; R and B are swapped).
pub fn raw_frame_from_rgb(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    let linelen = width * 3;
    let avipadwidth = pad(linelen, AVI_LINE_PADDING);
    let mut out = vec![0u8; avipadwidth * height];
    for y in 0..height {
        let src = &rgb[(height - 1 - y) * linelen..(height - y) * linelen];
        let dst = &mut out[y * avipadwidth..y * avipadwidth + linelen];
        for (d, s) in dst.chunks_exact_mut(3).zip(src.chunks_exact(3)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_frames() {
        assert_eq!(video_file_name(Some("demo1"), |_| false).unwrap(), "videos/demo1.avi");
        assert_eq!(video_file_name(None, |f| f == "videos/video0000.avi").unwrap(), "videos/video0001.avi");
        let rgb = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        // 2x2: rows flipped, BGR, 6 -> 8 bytes a line
        assert_eq!(raw_frame_from_rgb(&rgb, 2, 2), vec![9, 8, 7, 12, 11, 10, 0, 0, 3, 2, 1, 6, 5, 4, 0, 0]);
        assert_eq!(pad(6, 4), 8);
        assert_eq!(padlen(5, 2), 1);
    }
}
