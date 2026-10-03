//! cl_avi.c (dec038): a short capture is a valid AVI. The RIFF tree is
//! walked: RIFF size, hdrl/avih (frame period, frame count, streams,
//! size), the vids and auds stream headers, the movi list size, every
//! idx1 entry pointing at its 00dc / 01wb chunk; ffprobe reads it when
//! installed.
use trem_sim::kernel::avi::{raw_frame_from_rgb, AviFile, DmaFormat};

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The chunks of a list body: (fourcc, offset of the data, size).
fn chunks(b: &[u8], mut at: usize, end: usize) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    while at + 8 <= end {
        let id = String::from_utf8_lossy(&b[at..at + 4]).into_owned();
        let size = u32le(b, at + 4) as usize;
        out.push((id, at + 8, size));
        at += 8 + size + (size & 1);
    }
    assert_eq!(at, end, "chunks fill their list");
    out
}

fn capture(path: &std::path::Path, mjpeg: bool) -> Vec<u8> {
    let (w, h) = (6, 4);
    let mut avi = AviFile::open(path, 25, w, h, mjpeg, Some(DmaFormat { speed: 44100, channels: 2, samplebits: 16 })).unwrap();
    assert!(avi.audio);
    for f in 0..10u8 {
        let frame = if mjpeg {
            // any bytes stand in for the JPEG of RE_SaveJPGToBuffer (odd length: padded)
            let mut j = vec![0xff, 0xd8];
            j.extend(std::iter::repeat_n(f, 31 + f as usize));
            j.extend([0xff, 0xd9]);
            j
        } else {
            let rgb: Vec<u8> = (0..(w * h * 3) as usize).map(|i| (i as u8).wrapping_add(f)).collect();
            raw_frame_from_rgb(&rgb, w as usize, h as usize)
        };
        avi.write_video_frame(&frame).unwrap();
        // S_GetSoundtime: the mixer's samples of this frame, 4 bytes each
        let n = avi.sound_frame_samples() as usize;
        assert_eq!(n, 1764);
        let pcm: Vec<u8> = (0..n * 4).map(|i| (i * 7) as u8).collect();
        avi.write_audio_frame(&pcm).unwrap();
        assert_eq!(avi.video_frame_msec(1.0), 40);
    }
    let out = avi.close().unwrap();
    assert!(out.contains("Wrote 10:10 frames to"), "{out}");
    std::fs::read(path).unwrap()
}

fn check(b: &[u8], mjpeg: bool) {
    assert_eq!(&b[0..4], b"RIFF");
    assert_eq!(u32le(b, 4) as usize, b.len() - 8, "RIFF size");
    assert_eq!(&b[8..12], b"AVI ");
    let top = chunks(b, 12, b.len());
    let ids: Vec<&str> = top.iter().map(|c| c.0.as_str()).collect();
    assert_eq!(ids, ["LIST", "LIST", "idx1"]);
    // hdrl
    let (_, hdrl, hsize) = top[0];
    assert_eq!(&b[hdrl..hdrl + 4], b"hdrl");
    let h = chunks(b, hdrl + 4, hdrl + hsize);
    assert_eq!(h[0].0, "avih");
    let avih = h[0].1;
    assert_eq!(u32le(b, avih), 40000, "dwMicroSecPerFrame");
    assert_eq!(u32le(b, avih + 12), 0x110, "HAS_INDEX | IS_INTERLEAVED");
    assert_eq!(u32le(b, avih + 16), 10, "dwTotalFrames");
    assert_eq!(u32le(b, avih + 24), 2, "dwStreams");
    assert_eq!((u32le(b, avih + 32), u32le(b, avih + 36)), (6, 4));
    // the video and audio stream lists
    assert_eq!(h[1].0, "LIST");
    assert_eq!(&b[h[1].1..h[1].1 + 4], b"strl");
    let vs = chunks(b, h[1].1 + 4, h[1].1 + h[1].2);
    assert_eq!(&b[vs[0].1..vs[0].1 + 4], b"vids");
    assert_eq!(&b[vs[0].1 + 4..vs[0].1 + 8], if mjpeg { b"MJPG" } else { &[0u8; 4] });
    assert_eq!(u32le(b, vs[0].1 + 32), 10, "strh dwDataLength");
    assert_eq!(vs[1].0, "strf");
    assert_eq!(u32le(b, vs[1].1 + 4), 6);
    let aus = chunks(b, h[2].1 + 4, h[2].1 + h[2].2);
    assert_eq!(&b[aus[0].1..aus[0].1 + 4], b"auds");
    let wf = aus[1].1;
    assert_eq!(u16::from_le_bytes([b[wf], b[wf + 1]]), 1, "WAVE_FORMAT_PCM");
    assert_eq!(u16::from_le_bytes([b[wf + 2], b[wf + 3]]), 2);
    assert_eq!(u32le(b, wf + 4), 44100);
    assert_eq!(u32le(b, aus[0].1 + 32), 17640, "audio dwDataLength in samples");
    // movi
    let (_, movi, msize) = top[1];
    assert_eq!(&b[movi..movi + 4], b"movi");
    let m = chunks(b, movi + 4, movi + msize);
    assert_eq!(m.iter().filter(|c| c.0 == "00dc").count(), 10);
    assert_eq!(m.iter().filter(|c| c.0 == "01wb").count(), 10);
    if !mjpeg {
        assert!(m.iter().filter(|c| c.0 == "00dc").all(|c| c.2 == 20 * 4), "BI_RGB lines of 18 bytes padded to 20");
    }
    // idx1: offsets from the "movi" fourcc
    let (_, idx, isize) = top[2];
    assert_eq!(isize, 20 * 16);
    for e in 0..20 {
        let at = idx + e * 16;
        let id = &b[at..at + 4];
        let off = u32le(b, at + 8) as usize;
        let len = u32le(b, at + 12) as usize;
        assert_eq!(&b[movi + off..movi + off + 4], id, "index entry {e}");
        assert_eq!(u32le(b, movi + off + 4) as usize, len);
        assert_eq!(u32le(b, at + 4), if id == b"00dc" { 0x10 } else { 0 });
    }
}

#[test]
fn short_capture_is_a_valid_avi() {
    let dir = std::env::temp_dir().join(format!("trem_avi_{}", std::process::id()));
    for mjpeg in [false, true] {
        let path = dir.join("videos").join(if mjpeg { "mjpg.avi" } else { "raw.avi" });
        let b = capture(&path, mjpeg);
        check(&b, mjpeg);
        if !mjpeg {
            // a real decoder agrees, when there is one
            if let Ok(o) = std::process::Command::new("ffprobe").args(["-v", "error", "-show_entries", "stream=codec_type,nb_frames", "-of", "csv=p=0"]).arg(&path).output() {
                let s = String::from_utf8_lossy(&o.stdout);
                assert!(o.status.success() && s.contains("video") && s.contains("audio"), "ffprobe: {s} {}", String::from_utf8_lossy(&o.stderr));
            }
        }
    }
    // cl_aviFrameRate 0 refuses
    assert!(AviFile::open(&dir.join("x.avi"), 0, 4, 4, true, None).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
