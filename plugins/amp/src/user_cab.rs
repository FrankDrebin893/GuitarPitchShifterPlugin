//! A player's own cabinet: an impulse response in a WAV file, made ready for the cabinet's
//! convolution. Everything in here reads files, allocates and takes its time: none of it
//! is for the audio thread. `cab_stage` hands the result over.
//!
//! The files live in one folder, `Hojt Audio/Cabinets` in the user's Documents folder, and
//! are picked by stepping through them in name order. There is no file dialog.
//!
//! What is done to a file, in this order:
//! 1. Read: mono or stereo (the left channel), 16, 24 or 32 bit integer or 32 bit float,
//!    8 to 384 kHz. Only the first two seconds are looked at
//! 2. Resampled to the host's rate with a windowed sinc, unless it is at that rate already
//! 3. Trimmed: it starts where it first reaches -60 dB of its peak, but never more than
//!    0.2 ms before it first reaches half its peak. So silence, and the ringing a linear
//!    phase response has in front of its peak, do not delay the sound
//! 4. Cut to `cab::USER_IR_MS`, the last quarter of that faded out, and what is below
//!    -80 dB of the peak at its end dropped
//! 5. Levelled like the cabinets designed here: 0 dB average power from 100 Hz to 4 kHz

use crate::cab::{ir_magnitude, level_gain, user_ir_len, USER_IR_MS};
use std::fs::File;
use std::io::{BufReader, ErrorKind};
use std::path::{Path, PathBuf};

/// Set this to a folder to have the cabinets read from there instead of from Documents
pub const FOLDER_ENV: &str = "HOJT_CABINETS_DIR";
// Below the Documents folder
const FOLDER: [&str; 2] = [suite_common::VENDOR, "Cabinets"];
const EXTENSION: &str = "wav";

// What the display shows for the amp's own cabinet, for a file that could not be used,
// and in front of the name of a file that is not there
pub const OWN_NAME: &str = "OWN";
pub const BAD_NAME: &str = "BAD FILE";
pub const MISSING_MARK: char = '?';
// A name too long for the display keeps its start and this many characters of its end,
// where takes and microphone positions are told apart, with this between them
const NAME_TAIL: usize = 4;
const NAME_CUT_MARK: char = '~';

const RATE_HZ: [u32; 2] = [8_000, 384_000];
// How much of a file is read
const READ_S: f64 = 2.0;

// The response starts where it first reaches this share of its peak
const START_GAIN: f32 = 0.001;
// And it has arrived where it first reaches this share
const ARRIVAL_GAIN: f32 = 0.5;
// The most that is kept in front of the arrival
pub const PRE_MS: f32 = 0.2;
// The end is dropped from where it stays below this share of the peak
const END_GAIN: f32 = 1e-4;
// Share of the longest response, at its end, that is faded to zero when a file is longer
const FADE_SHARE: f32 = 0.25;
// Read around the response before it is resampled: the resampler rings on both sides
const LEAD_MS: f64 = 2.0;
const TAIL_MS: f64 = 5.0;
// A response that needs more gain than this to be levelled has nothing in the band
const LEVEL_GAIN_MAX: f32 = 1e4;

// The resampler: a sinc with this many zero crossings either side, under a Kaiser window
// that keeps what would fold back 90 dB down. Its corner sits this far below half the
// lower of the two rates, so that it has fallen that far there: flat to 0.41 of the rate
const ZERO_CROSSINGS: f64 = 32.0;
const KAISER_BETA: f64 = 9.0;
const CUTOFF_SHARE: f64 = 0.91;

/// Why a file gave no cabinet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// There is no such file
    Missing,
    /// Not a WAV file this reads, or nothing in it
    Bad,
}

/// One channel of a WAV file
pub struct Recording {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Where the cabinets are: `FOLDER_ENV` if set, else `Hojt Audio/Cabinets` in Documents.
/// Looks nothing up on the disk and creates nothing
pub fn cabinets_dir() -> Option<PathBuf> {
    match std::env::var_os(FOLDER_ENV) {
        Some(folder) if !folder.is_empty() => Some(PathBuf::from(folder)),
        _ => documents_dir().map(|documents| FOLDER.iter().fold(documents, |path, part| path.join(part))),
    }
}

/// The user's Documents folder as Windows knows it, which follows it when it was moved
/// (to a cloud drive, for one). Failing that, the one in the profile
#[cfg(windows)]
fn documents_dir() -> Option<PathBuf> {
    known_documents_dir().or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("Documents")))
}

#[cfg(not(windows))]
fn documents_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|home| !home.is_empty()).map(|home| PathBuf::from(home).join("Documents"))
}

#[cfg(windows)]
fn known_documents_dir() -> Option<PathBuf> {
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::OsStringExt;

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);

    #[link(name = "shell32")]
    extern "system" {
        fn SHGetKnownFolderPath(id: *const Guid, flags: u32, token: *mut c_void, path: *mut *mut u16) -> i32;
    }
    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(memory: *mut c_void);
    }

    // FOLDERID_Documents
    const DOCUMENTS: Guid = Guid(0xFDD3_9AD0, 0x238F, 0x46AF, [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7]);
    // KF_FLAG_DONT_VERIFY: the path as configured, without touching the folder
    const DONT_VERIFY: u32 = 0x4000;

    let mut wide: *mut u16 = std::ptr::null_mut();
    // SAFETY: the function writes one pointer to a null-terminated string into `wide`,
    // which is read up to that null and freed with the function the API names for it,
    // whether the call succeeded or not
    unsafe {
        let result = SHGetKnownFolderPath(&DOCUMENTS, DONT_VERIFY, std::ptr::null_mut(), &mut wide);
        let path = (result >= 0 && !wide.is_null()).then(|| {
            let len = (0..).take_while(|&index| *wide.add(index) != 0).count();
            PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(wide, len)))
        });
        CoTaskMemFree(wide.cast());
        path.filter(|path| !path.as_os_str().is_empty())
    }
}

/// True for a plain file name: what is stored with a project never leads out of the folder
pub fn is_file_name(name: &str) -> bool {
    !name.is_empty() && Path::new(name).file_name().is_some_and(|file| file == name)
}

/// The WAV files in a folder, by name. Nothing when there is no such folder
pub fn list(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| Path::new(name).extension().is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION)))
        .collect();
    files.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
    files
}

/// The choice before or after `current` in the round of the amp's own cabinet, which is
/// the empty name, and then the files. A `current` that is not among the files counts as
/// the amp's own
pub fn neighbour(files: &[String], current: &str, forward: bool) -> String {
    let count = files.len() + 1;
    let position = files.iter().position(|file| file == current).map_or(0, |index| index + 1);
    let next = (position + if forward { 1 } else { count - 1 }) % count;
    if next == 0 {
        String::new()
    } else {
        files[next - 1].clone()
    }
}

/// A file's name as the display shows it: without `.wav`, in capitals, in letters the
/// display's font has, and no longer than `max_chars`
pub fn display_name(file: &str, max_chars: usize) -> String {
    let path = Path::new(file);
    let has_extension = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION));
    let stem = if has_extension { path.file_stem().and_then(|stem| stem.to_str()).unwrap_or(file) } else { file };
    let shown: Vec<char> = stem
        .to_uppercase()
        .chars()
        .map(|letter| {
            // The font has Latin letters with their accents, digits and the usual signs
            let printable = (letter.is_alphanumeric() && (letter as u32) < 0x180) || " -_.+#&()".contains(letter);
            if printable {
                letter
            } else {
                '-'
            }
        })
        .collect();
    if shown.len() <= max_chars {
        return shown.into_iter().collect();
    }
    let tail = NAME_TAIL.min(max_chars.saturating_sub(2));
    let head = max_chars.saturating_sub(tail + 1);
    shown[..head].iter().chain([&NAME_CUT_MARK]).chain(&shown[shown.len() - tail..]).collect()
}

/// Reads the start of the left channel of a WAV file
pub fn read_wav(path: &Path) -> Result<Recording, LoadError> {
    let file = File::open(path).map_err(|error| match error.kind() {
        ErrorKind::NotFound => LoadError::Missing,
        _ => LoadError::Bad,
    })?;
    let mut reader = hound::WavReader::new(BufReader::new(file)).map_err(|_| LoadError::Bad)?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    if !(1..=2).contains(&channels) || !(RATE_HZ[0]..=RATE_HZ[1]).contains(&spec.sample_rate) {
        return Err(LoadError::Bad);
    }

    // A file that ends before its header says it does gives what it has
    let wanted = (READ_S * spec.sample_rate as f64) as usize * channels;
    let samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => {
            reader.samples::<f32>().take(wanted).map_while(Result::ok).step_by(channels).collect()
        }
        (hound::SampleFormat::Int, bits @ (16 | 24 | 32)) => {
            let scale = 1.0 / (1_i64 << (bits - 1)) as f64;
            reader
                .samples::<i32>()
                .take(wanted)
                .map_while(Result::ok)
                .step_by(channels)
                .map(|sample| (sample as f64 * scale) as f32)
                .collect()
        }
        _ => return Err(LoadError::Bad),
    };
    Ok(Recording {
        samples,
        sample_rate: spec.sample_rate,
    })
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |top, sample| top.max(sample.abs()))
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let phase = std::f64::consts::PI * x;
        phase.sin() / phase
    }
}

/// Modified Bessel function of the first kind and order zero, for the Kaiser window
fn bessel_i0(x: f64) -> f64 {
    let quarter = x * x / 4.0;
    let (mut sum, mut term) = (1.0, 1.0);
    for step in 1..60 {
        term *= quarter / (step * step) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// A signal at another sample rate, as many seconds long. Band-limited to just under half
/// the lower of the two rates. The first sample stays the first: nothing is delayed
pub fn resample(input: &[f32], from_rate: f64, to_rate: f64) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    // Input samples per output sample, and the lower rate as a share of the input's
    let step = from_rate / to_rate;
    let low = (to_rate / from_rate).min(1.0);
    let cutoff = low * CUTOFF_SHARE;
    let half_width = ZERO_CROSSINGS / low;
    let window_scale = 1.0 / bessel_i0(KAISER_BETA);

    let len = (input.len() as f64 / step).ceil() as usize;
    (0..len)
        .map(|index| {
            let centre = index as f64 * step;
            let first = (centre - half_width).ceil().max(0.0) as usize;
            let last = ((centre + half_width).floor() as usize).min(input.len() - 1);
            let mut sum = 0.0;
            for (offset, &sample) in input[first..=last].iter().enumerate() {
                let distance = centre - (first + offset) as f64;
                let position = distance / half_width;
                let window = bessel_i0(KAISER_BETA * (1.0 - position * position).max(0.0).sqrt()) * window_scale;
                sum += sample as f64 * cutoff * sinc(cutoff * distance) * window;
            }
            sum as f32
        })
        .collect()
}

/// Where a response starts: at its first sample of `START_GAIN` of its peak, or `PRE_MS`
/// before its first of `ARRIVAL_GAIN`, whichever is later. With it, where that arrival is,
/// and whether something of the response is left in front of the start
fn find_start(response: &[f32], sample_rate: f32) -> Option<(usize, usize, bool)> {
    let top = peak(response);
    if !(top > 0.0 && top.is_finite()) {
        return None;
    }
    let first = response.iter().position(|sample| sample.abs() >= top * START_GAIN)?;
    let arrival = response.iter().position(|sample| sample.abs() >= top * ARRIVAL_GAIN)?;
    let pre = (PRE_MS * 0.001 * sample_rate).round() as usize;
    let start = first.max(arrival.saturating_sub(pre));
    Some((start, arrival, start > first))
}

/// Steps 3 to 5 of what is done to a file: a response at the host's rate, trimmed, cut to
/// length and levelled
pub fn trim_and_level(response: &[f32], sample_rate: f32) -> Result<Vec<f32>, LoadError> {
    if !response.iter().all(|sample| sample.is_finite()) {
        return Err(LoadError::Bad);
    }
    let (start, arrival, cut) = find_start(response, sample_rate).ok_or(LoadError::Bad)?;
    let top = peak(response);
    let mut taps = response[start..].to_vec();
    // Something was cut off in front: what is left of it comes in gradually
    if cut {
        let rise = arrival - start;
        for (index, tap) in taps[..rise].iter_mut().enumerate() {
            let position = (index + 1) as f64 / (rise + 1) as f64;
            *tap *= (0.5 - 0.5 * (std::f64::consts::PI * position).cos()) as f32;
        }
    }

    let longest = user_ir_len(sample_rate);
    if taps.len() > longest {
        taps.truncate(longest);
        let fade_start = ((1.0 - FADE_SHARE) * longest as f32) as usize;
        for (index, tap) in taps.iter_mut().enumerate().skip(fade_start) {
            let position = (index + 1 - fade_start) as f64 / (longest - fade_start) as f64;
            *tap *= (0.5 + 0.5 * (std::f64::consts::PI * position).cos()) as f32;
        }
    }
    let last = taps.iter().rposition(|sample| sample.abs() >= top * END_GAIN).ok_or(LoadError::Bad)?;
    taps.truncate(last + 1);

    let gain = level_gain(|freq_hz| ir_magnitude(&taps, freq_hz, sample_rate));
    if !(gain.is_finite() && gain <= LEVEL_GAIN_MAX) {
        return Err(LoadError::Bad);
    }
    for tap in &mut taps {
        *tap *= gain;
    }
    Ok(taps)
}

/// Steps 2 to 5: the taps of a recording for the cabinet at the host's sample rate
pub fn prepare(recording: &Recording, sample_rate: f32) -> Result<Vec<f32>, LoadError> {
    let source = &recording.samples;
    if !source.iter().all(|sample| sample.is_finite()) {
        return Err(LoadError::Bad);
    }
    let top = peak(source);
    let first = source.iter().position(|sample| top > 0.0 && sample.abs() >= top * START_GAIN).ok_or(LoadError::Bad)?;
    if recording.sample_rate as f32 == sample_rate {
        return trim_and_level(&source[first..], sample_rate);
    }

    // Only what can end up in the cabinet is resampled, with some room around it
    let from_rate = recording.sample_rate as f64;
    let begin = first.saturating_sub((LEAD_MS * 0.001 * from_rate) as usize);
    let end = (first + ((USER_IR_MS as f64 + TAIL_MS) * 0.001 * from_rate) as usize).min(source.len());
    trim_and_level(&resample(&source[begin..end], from_rate, sample_rate as f64), sample_rate)
}

/// The taps of a WAV file for the cabinet at the host's sample rate
pub fn load(path: &Path, sample_rate: f32) -> Result<Vec<f32>, LoadError> {
    prepare(&read_wav(path)?, sample_rate)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::amp::model::Amp;
    use crate::cab::{design_ir, CabIr, Cabinet};
    use crate::test_util::{to_db, Noise};

    /// A folder for the files of one test, below `target`, empty
    pub fn test_dir(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test_cabinets").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes a WAV file with the same samples in every channel but the first, which
    /// gets `left`: the others are there to be ignored
    pub fn write_ir_wav(path: &Path, left: &[f32], channels: u16, bits: u16, float: bool, sample_rate: u32) {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: bits,
            sample_format: if float { hound::SampleFormat::Float } else { hound::SampleFormat::Int },
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &sample in left {
            for channel in 0..channels {
                let sample = if channel == 0 { sample } else { -0.5 * sample };
                if float {
                    writer.write_sample(sample).unwrap();
                } else {
                    let scale = ((1_i64 << (bits - 1)) - 1) as f64;
                    writer.write_sample((sample.clamp(-1.0, 1.0) as f64 * scale).round() as i32).unwrap();
                }
            }
        }
        writer.finalize().unwrap();
    }

    /// A cabinet designed here as one impulse response, resonance included: what a file
    /// of it holds. Peaks at about `level`
    pub fn whole_response(amp: Amp, sample_rate: f32, seconds: f32, level: f32) -> Vec<f32> {
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(sample_rate);
        cabinet.set_ir(&design_ir(&amp.model().cab, sample_rate));
        let mut response = vec![0.0; (seconds * sample_rate) as usize];
        response[0] = 1.0;
        cabinet.process(&mut response);
        let top = peak(&response);
        response.iter().map(|sample| sample * level / top).collect()
    }

    // The band the responses are held against each other in. The cabinets designed here
    // are only the same at every sample rate up to `DESIGN_TOP_HZ`: above it they are far
    // down their roll-off, where a filter designed at 48 kHz bends away from the same one
    // designed at 192 kHz (3 dB at 8 kHz for the steepest). That is the designs, not the
    // resampler, which is held to the whole band against the response it was given
    pub const BAND_HZ: [f32; 2] = [60.0, 8000.0];
    const DESIGN_TOP_HZ: f32 = 4000.0;

    /// Largest difference in dB between two responses, given as their gain at a frequency,
    /// over `probes` frequencies from 60 Hz to `top_hz`, and the frequency it is at
    fn largest_gap_db(a: impl Fn(f32) -> f32, b: impl Fn(f32) -> f32, top_hz: f32, probes: usize) -> (f32, f32) {
        let mut worst = (0.0f32, 0.0);
        for probe in 0..probes {
            let freq_hz = BAND_HZ[0] * (top_hz / BAND_HZ[0]).powf(probe as f32 / (probes - 1) as f32);
            let difference = to_db(a(freq_hz) / b(freq_hz));
            if difference.abs() > worst.0.abs() {
                worst = (difference, freq_hz);
            }
        }
        worst
    }

    /// The same for two impulse responses, each at its own sample rate
    pub fn largest_difference_db(a: (&[f32], f32), b: (&[f32], f32), top_hz: f32, probes: usize) -> (f32, f32) {
        largest_gap_db(|freq_hz| ir_magnitude(a.0, freq_hz, a.1), |freq_hz| ir_magnitude(b.0, freq_hz, b.1), top_hz, probes)
    }

    /// And for what a file gives once loaded, against the built-in cabinet with its filter
    fn off_the_built_in_db(taps: &[f32], amp: Amp, sample_rate: f32, top_hz: f32, probes: usize) -> (f32, f32) {
        let built_in: CabIr = design_ir(&amp.model().cab, sample_rate);
        largest_gap_db(
            |freq_hz| ir_magnitude(taps, freq_hz, sample_rate),
            |freq_hz| built_in.magnitude(freq_hz, sample_rate),
            top_hz,
            probes,
        )
    }

    /// Average level of a response over the band the cabinets are levelled in, in dB
    fn level_db(taps: &[f32], sample_rate: f32) -> f32 {
        -to_db(level_gain(|freq_hz| ir_magnitude(taps, freq_hz, sample_rate)))
    }

    fn arrival(taps: &[f32]) -> usize {
        let top = peak(taps);
        taps.iter().position(|sample| sample.abs() >= top * ARRIVAL_GAIN).unwrap()
    }

    #[test]
    fn test_every_accepted_format_reads_the_same() {
        let dir = test_dir("formats");
        let response = whole_response(Amp::Brol, 48000.0, 0.02, 0.8);
        let reference = trim_and_level(&response, 48000.0).unwrap();
        for (bits, float) in [(16, false), (24, false), (32, false), (32, true)] {
            for channels in [1, 2] {
                let path = dir.join(format!("{bits}_{float}_{channels}.wav"));
                write_ir_wav(&path, &response, channels, bits, float, 48000);
                let recording = read_wav(&path).unwrap();
                assert_eq!(recording.sample_rate, 48000);
                assert_eq!(recording.samples.len(), response.len(), "{} bit, {} channels", bits, channels);
                // The left channel, and not its mix with the other one
                let off = recording.samples.iter().zip(&response).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
                assert!(off < 2.0 / 32768.0, "{} bit, {} channels: off by {}", bits, channels, off);

                let taps = load(&path, 48000.0).unwrap();
                let (difference, freq_hz) = largest_difference_db((&taps, 48000.0), (&reference, 48000.0), BAND_HZ[1], 60);
                assert!(difference.abs() < 0.2, "{} bit, {} channels: {:.2} dB at {:.0} Hz", bits, channels, difference, freq_hz);
            }
        }
    }

    #[test]
    fn test_bad_and_missing_files_are_told_apart() {
        let dir = test_dir("rejected");
        assert_eq!(load(&dir.join("nothing.wav"), 48000.0).err(), Some(LoadError::Missing));

        let text = dir.join("text.wav");
        std::fs::write(&text, b"this is not a recording").unwrap();
        assert_eq!(load(&text, 48000.0).err(), Some(LoadError::Bad));
        std::fs::write(&text, b"").unwrap();
        assert_eq!(load(&text, 48000.0).err(), Some(LoadError::Bad));

        let response = whole_response(Amp::Brol, 48000.0, 0.01, 0.8);
        // 8 bit, three channels, a rate nothing records at, silence, no samples at all
        let cases: [(&str, &[f32], u16, u16, u32); 5] = [
            ("eight", &response, 1, 8, 48000),
            ("three", &response, 3, 16, 48000),
            ("slow", &response, 1, 16, 4000),
            ("silent", &[0.0; 480], 1, 16, 48000),
            ("empty", &[], 1, 16, 48000),
        ];
        for (name, samples, channels, bits, sample_rate) in cases {
            let path = dir.join(format!("{name}.wav"));
            write_ir_wav(&path, samples, channels, bits, false, sample_rate);
            assert_eq!(load(&path, 48000.0).err(), Some(LoadError::Bad), "{}", name);
        }

        let mut broken = response.clone();
        broken[7] = f32::NAN;
        let path = dir.join("nan.wav");
        write_ir_wav(&path, &broken, 1, 32, true, 48000);
        assert_eq!(load(&path, 48000.0).err(), Some(LoadError::Bad));
    }

    #[test]
    fn test_a_file_cut_short_gives_what_it_has() {
        let dir = test_dir("cut_short");
        let response = whole_response(Amp::Klar, 48000.0, 0.02, 0.8);
        let path = dir.join("whole.wav");
        write_ir_wav(&path, &response, 1, 16, false, 48000);
        let bytes = std::fs::read(&path).unwrap();
        let cut = dir.join("cut.wav");
        std::fs::write(&cut, &bytes[..bytes.len() - 601]).unwrap();
        let recording = read_wav(&cut).unwrap();
        assert_eq!(recording.samples.len(), response.len() - 301);
    }

    #[test]
    fn test_resampler_keeps_the_response() {
        // The same filter seen at two rates: what the resampler itself changes
        for amp in Amp::ALL {
            let original = design_ir(&amp.model().cab, 48000.0).taps;
            for to_rate in [44100.0, 88200.0, 96000.0, 192000.0] {
                let resampled = resample(&original, 48000.0, to_rate as f64);
                let scaled: Vec<f32> = resampled.iter().map(|sample| sample * 48000.0 / to_rate).collect();
                let (difference, freq_hz) = largest_difference_db((&scaled, to_rate), (&original, 48000.0), BAND_HZ[1], 80);
                assert!(difference.abs() < 0.05, "{:?} to {} Hz: {:.3} dB at {:.0} Hz", amp, to_rate, difference, freq_hz);
            }
        }
    }

    #[test]
    fn test_resampled_cabinet_matches_the_one_designed_at_that_rate() {
        for amp in Amp::ALL {
            let original = design_ir(&amp.model().cab, 48000.0).taps;
            for to_rate in [44100.0, 96000.0, 192000.0] {
                let native = design_ir(&amp.model().cab, to_rate).taps;
                let native = trim_and_level(&native, to_rate).unwrap();
                let resampled = trim_and_level(&resample(&original, 48000.0, to_rate as f64), to_rate).unwrap();
                let (difference, freq_hz) = largest_difference_db((&resampled, to_rate), (&native, to_rate), DESIGN_TOP_HZ, 80);
                assert!(difference.abs() < 0.5, "{:?} to {} Hz: {:.2} dB at {:.0} Hz", amp, to_rate, difference, freq_hz);
            }
        }
    }

    #[test]
    fn test_resampler_goes_down_as_well_as_up() {
        // From a high rate what is above the new half rate has to go, not fold back
        let mut noise = Noise::new(3);
        let wide: Vec<f32> = (0..9600).map(|index| if index < 4800 { noise.next() } else { 0.0 }).collect();
        let down = resample(&wide, 192000.0, 48000.0);
        assert_eq!(down.len(), 2400);
        // A tone the lower rate cannot hold leaves next to nothing
        let tone = |freq_hz: f64, sample_rate: f64, index: usize| (std::f64::consts::TAU * freq_hz * index as f64 / sample_rate).sin() as f32;
        let high: Vec<f32> = (0..9600).map(|index| tone(30000.0, 192000.0, index)).collect();
        let folded = resample(&high, 192000.0, 48000.0);
        assert!(to_db(peak(&folded[300..2100])) < -80.0, "{:.1} dB", to_db(peak(&folded[300..2100])));
        // And one it can hold comes through as it was
        let low: Vec<f32> = (0..9600).map(|index| tone(5000.0, 192000.0, index)).collect();
        let kept = resample(&low, 192000.0, 48000.0);
        assert!(to_db(peak(&kept[300..2100])).abs() < 0.01, "{:.3} dB", to_db(peak(&kept[300..2100])));
        for (index, &sample) in kept.iter().enumerate().take(2100).skip(300).step_by(37) {
            let expected = tone(5000.0, 48000.0, index);
            assert!((sample - expected).abs() < 1e-4, "Sample {}: {} for {}", index, sample, expected);
        }
    }

    #[test]
    fn test_silence_in_front_is_trimmed_and_the_peak_is_not_delayed() {
        let dir = test_dir("trimmed");
        for from_rate in [44100, 48000, 96000] {
            // 30 ms of nothing, then the cabinet
            let mut response = vec![0.0; (0.03 * from_rate as f32) as usize];
            response.extend(whole_response(Amp::Torden, from_rate as f32, 0.03, 0.7));
            let path = dir.join(format!("late_{from_rate}.wav"));
            write_ir_wav(&path, &response, 1, 24, false, from_rate);
            for sample_rate in [44100.0, 48000.0, 192000.0] {
                let taps = load(&path, sample_rate).unwrap();
                let limit = (PRE_MS * 0.001 * sample_rate).round() as usize;
                assert!(arrival(&taps) <= limit, "{} to {} Hz: arrives at sample {}", from_rate, sample_rate, arrival(&taps));
            }
        }
    }

    #[test]
    fn test_ringing_in_front_of_the_peak_is_cut_to_a_fifth_of_a_millisecond() {
        // A linear phase response: as much in front of its peak as behind it
        let sample_rate = 48000.0;
        let centre = 480;
        let response: Vec<f32> = (0..961)
            .map(|index| {
                let distance = index as f64 - centre as f64;
                (0.35 * sinc(0.35 * distance) * (0.5 + 0.5 * (std::f64::consts::PI * distance / 481.0).cos())) as f32
            })
            .collect();
        let taps = trim_and_level(&response, sample_rate).unwrap();
        let limit = (PRE_MS * 0.001 * sample_rate).round() as usize;
        assert_eq!(arrival(&taps), limit);
        // What is left in front comes in from nothing
        assert!(taps[0].abs() < 0.1 * peak(&taps), "First tap: {} of {}", taps[0], peak(&taps));

        // A response that starts at once is left as it is, to the sample
        let direct = design_ir(&Amp::Brol.model().cab, sample_rate).taps;
        let taps = trim_and_level(&direct, sample_rate).unwrap();
        let gain = taps[0] / direct[0];
        let kept = &direct[..taps.len()];
        assert!(taps.iter().zip(kept).all(|(tap, sample)| (tap - sample * gain).abs() < 1e-6));
    }

    #[test]
    fn test_long_responses_are_cut_and_faded_and_short_ones_left_alone() {
        for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
            // Noise that dies away over a quarter of a second: far longer than the cabinet holds
            let mut noise = Noise::new(9);
            let long: Vec<f32> =
                (0..(0.3 * sample_rate) as usize).map(|index| noise.next() * (-(index as f32) / (0.05 * sample_rate)).exp()).collect();
            let mut long = long;
            long[0] = 1.0;
            let taps = trim_and_level(&long, sample_rate).unwrap();
            assert!(taps.len() <= user_ir_len(sample_rate), "{} taps at {} Hz", taps.len(), sample_rate);
            assert!(taps.len() > user_ir_len(sample_rate) * 9 / 10);
            assert!(taps.last().unwrap().abs() < 1e-3 * peak(&taps));
            // The first three quarters are the file's own
            let gain = taps[0] / long[0];
            let untouched = user_ir_len(sample_rate) * 3 / 4;
            assert!(taps[..untouched].iter().zip(&long).all(|(tap, sample)| (tap - sample * gain).abs() < 1e-4 * gain));
        }

        // A short one keeps its length, but not the silence behind it
        let mut short = design_ir(&Amp::Klar.model().cab, 48000.0).taps;
        let len = short.len();
        short.extend([0.0; 5000]);
        let taps = trim_and_level(&short, 48000.0).unwrap();
        assert!(taps.len() <= len && taps.len() > len / 2, "{} taps of {}", taps.len(), len);
    }

    #[test]
    fn test_every_response_comes_out_at_the_level_of_the_built_in_ones() {
        for sample_rate in [44100.0, 48000.0, 96000.0] {
            for amp in Amp::ALL {
                for level in [0.02, 1.0] {
                    let response = whole_response(amp, 48000.0, 0.1, level);
                    let recording = Recording {
                        samples: response,
                        sample_rate: 48000,
                    };
                    let taps = prepare(&recording, sample_rate).unwrap();
                    let level_db = level_db(&taps, sample_rate);
                    assert!(level_db.abs() < 0.01, "{:?} at {} Hz: {:.3} dB", amp, sample_rate, level_db);
                    // Which is the level of the cabinet it was made from, as designed
                    let designed = design_ir(&amp.model().cab, sample_rate);
                    let designed_db = -to_db(level_gain(|freq_hz| designed.magnitude(freq_hz, sample_rate)));
                    assert!(designed_db.abs() < 0.01);
                }
            }
        }
    }

    #[test]
    fn test_a_single_sample_is_a_cabinet_that_does_nothing() {
        let recording = Recording {
            samples: vec![0.0, 0.0, 0.25, 0.0, 0.0],
            sample_rate: 48000,
        };
        assert_eq!(prepare(&recording, 48000.0).unwrap(), vec![1.0]);
    }

    #[test]
    fn test_exported_cabinet_loaded_again_matches_the_built_in_one() {
        // What `export_cabinets` writes, read back: the built-in cabinet within half a dB
        let dir = test_dir("round_trip");
        for amp in Amp::ALL {
            let path = dir.join(format!("{}.wav", amp.model().name));
            write_ir_wav(&path, &whole_response(amp, 48000.0, EXPORT_S, EXPORT_LEVEL), 1, 24, false, 48000);
            for sample_rate in [44100.0, 48000.0, 96000.0] {
                // At the rate of the file the whole band; at another, as far up as the
                // built-in one is the same cabinet there
                let top_hz = if sample_rate == 48000.0 { BAND_HZ[1] } else { DESIGN_TOP_HZ };
                let taps = load(&path, sample_rate).unwrap();
                let (difference, freq_hz) = off_the_built_in_db(&taps, amp, sample_rate, top_hz, 80);
                assert!(difference.abs() < 0.5, "{:?} at {} Hz: {:.2} dB at {:.0} Hz", amp, sample_rate, difference, freq_hz);
            }
        }
    }

    #[test]
    fn test_files_are_listed_by_name_and_stepped_through_in_a_round() {
        let dir = test_dir("listed");
        assert!(list(&dir.join("not there")).is_empty());
        assert!(list(&dir).is_empty());
        for name in ["beta.wav", "Alpha.WAV", "gamma.Wav", "notes.txt", "delta.wav.bak"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        std::fs::create_dir(dir.join("folder.wav")).unwrap();
        let files = list(&dir);
        assert_eq!(files, ["Alpha.WAV", "beta.wav", "gamma.Wav"]);

        // The amp's own cabinet, then the files, and round again, both ways
        assert_eq!(neighbour(&files, "", true), "Alpha.WAV");
        assert_eq!(neighbour(&files, "Alpha.WAV", true), "beta.wav");
        assert_eq!(neighbour(&files, "gamma.Wav", true), "");
        assert_eq!(neighbour(&files, "", false), "gamma.Wav");
        assert_eq!(neighbour(&files, "Alpha.WAV", false), "");
        // From a file that is gone, as from the amp's own
        assert_eq!(neighbour(&files, "gone.wav", true), "Alpha.WAV");
        assert_eq!(neighbour(&files, "gone.wav", false), "gamma.Wav");
        // With no files there is nowhere to go
        assert_eq!(neighbour(&[], "", true), "");
        assert_eq!(neighbour(&[], "gone.wav", false), "");
    }

    #[test]
    fn test_names_are_shortened_for_the_display() {
        assert_eq!(display_name("Garage.wav", 10), "GARAGE");
        assert_eq!(display_name("ten chars.WAV", 10), "TEN CHARS");
        assert_eq!(display_name("exactly_10.wav", 10), "EXACTLY_10");
        // Too long: the start, and the end where the takes are numbered
        assert_eq!(display_name("Big box edge of cone 2.wav", 10), "BIG B~NE 2");
        assert_eq!(display_name("Big box edge of cone 3.wav", 10), "BIG B~NE 3");
        assert_eq!(display_name("Big box edge of cone 3.wav", 9), "BIG ~NE 3");
        // Letters the font has stay, others do not turn into empty boxes
        assert_eq!(display_name("Brøl_æå.wav", 10), "BRØL_ÆÅ");
        assert_eq!(display_name("箱 one;two*.wav", 10), "- ONE-TWO-");
        assert_eq!(display_name("no extension", 20), "NO EXTENSION");
        for name in ["a.wav", "Big box edge of cone 2.wav", "ß-ß-ß-ß-ß-ß-ß.wav"] {
            assert!(display_name(name, 10).chars().count() <= 10);
        }
    }

    #[test]
    fn test_only_plain_file_names_are_accepted() {
        assert!(is_file_name("cab.wav") && is_file_name("my cab 2.wav"));
        for name in ["", "..", "../cab.wav", "sub/cab.wav", "sub\\cab.wav", "C:\\cab.wav", "/cab.wav"] {
            assert!(!is_file_name(name) || cfg!(not(windows)) && name.contains('\\'), "{:?}", name);
        }
    }

    #[test]
    fn test_cabinets_folder_is_in_documents_unless_overridden() {
        // Reads the environment and asks the system; touches nothing on the disk
        let documents = documents_dir().expect("No Documents folder");
        assert!(documents.is_absolute(), "{}", documents.display());
        if std::env::var_os(FOLDER_ENV).is_none() {
            let dir = cabinets_dir().unwrap();
            assert!(dir.ends_with("Hojt Audio/Cabinets"), "{}", dir.display());
            assert!(dir.starts_with(&documents));
        }
    }

    // What `export_cabinets` writes: long enough for the resonance to ring out
    const EXPORT_S: f32 = 0.1;
    const EXPORT_LEVEL: f32 = 0.5;

    /// Writes the three built-in cabinets as 48 kHz WAV files to target/renders/cabinets:
    /// something to put in the cabinets folder and to hold other files against
    ///   cargo test -p amp --release export_cabinets -- --ignored --nocapture
    #[test]
    #[ignore]
    fn export_cabinets() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders/cabinets");
        std::fs::create_dir_all(&dir).unwrap();
        for amp in Amp::ALL {
            let path = dir.join(format!("{} cabinet.wav", amp.model().name));
            write_ir_wav(&path, &whole_response(amp, 48000.0, EXPORT_S, EXPORT_LEVEL), 1, 24, false, 48000);
            let taps = load(&path, 48000.0).unwrap();
            println!("{:<28}{:>6} taps when loaded at 48 kHz", path.file_name().unwrap().to_string_lossy(), taps.len());
        }
        println!("Wrote cabinets to {}", dir.display());
    }

    /// Prints what the resampler changes, and what a file of a built-in cabinet is off by
    /// once it has been loaded at each sample rate:
    ///   cargo test -p amp --release user_cab_report -- --ignored --nocapture
    /// which also runs `user_cab_report_cost` in `chain.rs`: the cost of the lengths and the latency
    #[test]
    #[ignore]
    fn user_cab_report_response() {
        println!("Resampler alone: largest change of a 48 kHz cabinet, 60 Hz to 8 kHz, in dB");
        println!("{:<8}{:>18}{:>18}{:>18}{:>18}", "", 44100, 88200, 96000, 192000);
        for amp in Amp::ALL {
            let original = design_ir(&amp.model().cab, 48000.0).taps;
            print!("{:<8}", amp.model().name);
            for to_rate in [44100.0, 88200.0, 96000.0, 192000.0f32] {
                let resampled = resample(&original, 48000.0, to_rate as f64);
                let scaled: Vec<f32> = resampled.iter().map(|sample| sample * 48000.0 / to_rate).collect();
                let (difference, freq_hz) = largest_difference_db((&scaled, to_rate), (&original, 48000.0), BAND_HZ[1], 600);
                print!("{:>8.4} at {:>4.0} Hz", difference, freq_hz);
            }
            println!();
        }
        println!();

        for top_hz in [DESIGN_TOP_HZ, BAND_HZ[1]] {
            println!("Resampled 48 kHz cabinet against the one designed at that rate, 60 Hz to {} Hz, in dB", top_hz);
            println!("{:<8}{:>18}{:>18}{:>18}", "", 44100, 96000, 192000);
            for amp in Amp::ALL {
                let original = design_ir(&amp.model().cab, 48000.0).taps;
                print!("{:<8}", amp.model().name);
                for to_rate in [44100.0, 96000.0, 192000.0f32] {
                    let native = trim_and_level(&design_ir(&amp.model().cab, to_rate).taps, to_rate).unwrap();
                    let resampled = trim_and_level(&resample(&original, 48000.0, to_rate as f64), to_rate).unwrap();
                    let (difference, freq_hz) = largest_difference_db((&resampled, to_rate), (&native, to_rate), top_hz, 600);
                    print!("{:>8.3} at {:>4.0} Hz", difference, freq_hz);
                }
                println!();
            }
            println!();
        }

        let dir = test_dir("report");
        for top_hz in [DESIGN_TOP_HZ, BAND_HZ[1]] {
            println!(
                "Built-in cabinet as a 48 kHz, 24 bit file, loaded: taps, and largest difference from the built-in one, 60 Hz to {} Hz",
                top_hz
            );
            println!("{:<8}{:>26}{:>26}{:>26}{:>26}", "", 44100, 48000, 96000, 192000);
            for amp in Amp::ALL {
                let path = dir.join(format!("{}.wav", amp.model().name));
                write_ir_wav(&path, &whole_response(amp, 48000.0, EXPORT_S, EXPORT_LEVEL), 1, 24, false, 48000);
                print!("{:<8}", amp.model().name);
                for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0f32] {
                    let taps = load(&path, sample_rate).unwrap();
                    let (difference, freq_hz) = off_the_built_in_db(&taps, amp, sample_rate, top_hz, 600);
                    print!("{:>6} taps{:>7.2} at {:>4.0} Hz", taps.len(), difference, freq_hz);
                }
                println!();
            }
            println!();
        }
    }
}
