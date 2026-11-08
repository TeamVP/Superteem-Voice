//! On-device speech to text with whisper.cpp. Audio never leaves the machine.

use crate::library;
use crate::process::RATE;
use std::io::{Read, Write};
use std::path::PathBuf;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const _: () = assert!(RATE == 16_000, "whisper needs 16 kHz audio");

const MODEL_FILE: &str = "ggml-base.en.bin";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";
const MODEL_MIN_BYTES: u64 = 100_000_000;
/// Clips shorter than this are read with a window cut to their length, several times faster.
const SHORT_CLIP_S: f32 = 20.0;
/// Whisper's encoder covers 30 s in 1500 positions.
const AUDIO_CTX_PER_S: f32 = 1500.0 / 30.0;
/// About 7.7 s, the shortest window that transcribed short commands without errors.
const MIN_AUDIO_CTX: f32 = 384.0;
/// Silence read after a short clip: a second normally, and more when whisper looped on the first read.
const QUICK_MARGIN_S: f32 = 1.0;
const RETRY_MARGIN_S: f32 = 8.0;
/// Context around each segment, because whisper does poorly on clipped words.
const PADDING_SAMPLES: usize = RATE as usize / 5;

pub fn model_path() -> PathBuf {
    library::models_dir().join(MODEL_FILE)
}

pub fn model_ready() -> bool {
    std::fs::metadata(model_path()).is_ok_and(|m| m.len() >= MODEL_MIN_BYTES)
}

/// Runs only when the person clicks download. Uses a temporary name so a failed download is never loaded.
/// `progress` gets the fraction done, when the server gives the size.
pub fn download_model(progress: impl Fn(f32)) -> Result<(), String> {
    let path = model_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let partial = path.with_extension("part");
    let result = (|| -> Result<u64, String> {
        let response = ureq::get(MODEL_URL).call().map_err(|e| e.to_string())?;
        let total: Option<u64> = response.header("Content-Length").and_then(|v| v.parse().ok());
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if let Some(total) = total {
                progress(done as f32 / total as f32);
            }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        Ok(done)
    })();
    match result {
        Ok(bytes) if bytes >= MODEL_MIN_BYTES => std::fs::rename(&partial, &path).map_err(|e| e.to_string()),
        other => {
            let _ = std::fs::remove_file(&partial);
            let reason = other.err().unwrap_or_else(|| "the file was incomplete".into());
            Err(format!("The speech model could not be downloaded: {reason}"))
        }
    }
}

/// Whisper labels non-speech as "[BLANK_AUDIO]" or "(wind blowing)", which is not what anyone said.
fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth as i32 - 1).max(0) as usize,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    collapse_repeats(&out.split_whitespace().collect::<Vec<_>>()).join(" ")
}

/// A run of words said this many times over is whisper stuck in a loop, not speech. A single word needs
/// more, as people do say "no, no, no".
const LOOP_REPEATS: usize = 3;
const LOOP_REPEATS_ONE_WORD: usize = 4;
const LOOP_MAX_WORDS: usize = 8;

/// Keeps one copy of a phrase whisper repeated over and over ("bookmark that bookmark that bookmark…"),
/// including the cut-off copy at the end where it ran out of tokens.
fn collapse_repeats<'a>(words: &[&'a str]) -> Vec<&'a str> {
    let key = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    let keys: Vec<String> = words.iter().map(|w| key(w)).collect();
    let mut out = Vec::new();
    let mut i = 0;
    'words: while i < words.len() {
        for n in 1..=LOOP_MAX_WORDS.min((words.len() - i) / LOOP_REPEATS) {
            let unit = &keys[i..i + n];
            let mut copies = 1;
            while i + (copies + 1) * n <= words.len() && keys[i + copies * n..i + (copies + 1) * n] == *unit {
                copies += 1;
            }
            if copies >= if n == 1 { LOOP_REPEATS_ONE_WORD } else { LOOP_REPEATS } {
                out.extend_from_slice(&words[i..i + n]);
                let mut next = i + copies * n;
                // A last copy cut short.
                let partial = keys[next..].iter().zip(unit).take_while(|(a, b)| a == b).count();
                if partial < n {
                    next += partial;
                }
                i = next;
                continue 'words;
            }
        }
        out.push(words[i]);
        i += 1;
    }
    out
}

pub struct Transcriber(WhisperContext);

impl Transcriber {
    pub fn load() -> Result<Self, String> {
        let path = model_path();
        let path = path.to_str().ok_or("The model path is not valid text.")?;
        WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map(Self)
            .map_err(|e| e.to_string())
    }

    /// Returns the words between two sample positions, or an empty string if there are none.
    pub fn transcribe(&self, pcm: &[i16], start: usize, end: usize) -> Result<String, String> {
        let from = start.saturating_sub(PADDING_SAMPLES).min(pcm.len());
        let to = (end + PADDING_SAMPLES).min(pcm.len());
        self.run(&pcm[from..to], None)
    }

    /// Transcribes all of `pcm`, with `prompt` as the words heard just before, to steer spelling.
    pub fn transcribe_with_prompt(&self, pcm: &[i16], prompt: &str) -> Result<String, String> {
        self.run(pcm, Some(prompt))
    }

    /// Transcribes all of `pcm` with nothing to steer it.
    pub fn transcribe_plain(&self, pcm: &[i16]) -> Result<String, String> {
        self.run(pcm, None)
    }

    fn run(&self, pcm: &[i16], prompt: Option<&str>) -> Result<String, String> {
        let mut audio: Vec<f32> = pcm.iter().map(|&s| s as f32 / 32768.0).collect();
        // Whisper rejects clips under a second, which a short command can be.
        audio.resize(audio.len().max(RATE as usize * 11 / 10), 0.0);
        let seconds = audio.len() as f32 / RATE as f32;
        if seconds < SHORT_CLIP_S {
            // Read quickly, whisper sometimes loops on a sentence until it runs out of tokens. A wider window
            // usually stops it at little cost, so only those clips are read again.
            for margin in [QUICK_MARGIN_S, RETRY_MARGIN_S] {
                let (text, capped) = self.decode(&audio, prompt, Some(margin))?;
                if !capped && !looped(&text) {
                    return Ok(clean(&text));
                }
            }
        }
        Ok(clean(&self.decode(&audio, prompt, None)?.0))
    }

    /// Returns the words, and whether whisper stopped only because it ran out of tokens. With `margin`, the
    /// window is cut to the clip plus that many seconds; without, whisper reads its full 30 s.
    fn decode(&self, audio: &[f32], prompt: Option<&str>, margin: Option<f32>) -> Result<(String, bool), String> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        params.set_n_threads(cores.clamp(1, 8) as i32);
        let seconds = audio.len() as f32 / RATE as f32;
        // Even fast speech is under 6 tokens a second, so more means whisper is stuck repeating itself.
        let max_tokens = (16.0 + 8.0 * seconds) as i32;
        if let Some(margin) = margin {
            // Whisper reads 30 s of audio whatever the clip. A short clip is read with a shorter window, which
            // is four times faster; much shorter than 8 s and whisper starts repeating itself or dropping words.
            let window = ((seconds + margin) * AUDIO_CTX_PER_S).ceil().clamp(MIN_AUDIO_CTX, 1500.0);
            params.set_audio_ctx(window as i32);
            params.set_single_segment(true);
            params.set_no_timestamps(true);
            // Retrying at higher temperatures when unsure costs seconds, and short clips rarely need it.
            params.set_temperature_inc(0.0);
            params.set_max_tokens(max_tokens);
        }
        if let Some(prompt) = prompt {
            params.set_initial_prompt(prompt);
        }
        params.set_language(Some("en"));
        params.set_no_context(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut state = self.0.create_state().map_err(|e| e.to_string())?;
        state.full(params, audio).map_err(|e| e.to_string())?;
        let mut tokens = 0;
        let text: String = state
            .as_iter()
            .map(|segment| {
                tokens += segment.n_tokens();
                segment.to_str_lossy().map(|t| t.into_owned()).unwrap_or_default()
            })
            .collect();
        Ok((text, margin.is_some() && tokens >= max_tokens))
    }
}

/// A sentence or more said three times running is whisper looping, not speech: people repeat a phrase for
/// emphasis, but rarely a whole sentence three times. Shorter loops are collapsed by `collapse_repeats`.
const SENTENCE_LOOP_MIN_WORDS: usize = 6;
const SENTENCE_LOOP_MAX_WORDS: usize = 40;

fn looped(text: &str) -> bool {
    let keys: Vec<String> =
        text.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).filter(|w| !w.is_empty()).collect();
    (SENTENCE_LOOP_MIN_WORDS..=SENTENCE_LOOP_MAX_WORDS).any(|n| {
        (0..keys.len().saturating_sub(3 * n - 1)).any(|i| keys[i..i + n] == keys[i + n..i + 2 * n] && keys[i..i + n] == keys[i + 2 * n..i + 3 * n])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_speech_labels_are_removed() {
        assert_eq!(clean(" [BLANK_AUDIO] "), "");
        assert_eq!(clean("(wind blowing) Hello  there [music]"), "Hello there");
        assert_eq!(clean("stray) words"), "stray words");
    }

    #[test]
    fn repetition_loops_are_collapsed() {
        let looped = format!("{}bookmark", "bookmark that ".repeat(15));
        assert_eq!(clean(&looped), "bookmark that");
        assert_eq!(clean("So, bookmark that. Bookmark that. Bookmark that. Then we ship."), "So, bookmark that. Then we ship.");
        assert_eq!(clean("no no no"), "no no no", "a word said three times is speech");
        assert_eq!(clean("go go go go go"), "go");
        assert_eq!(clean("that that is it"), "that that is it");
    }

    #[test]
    fn a_sentence_said_three_times_running_is_a_loop() {
        let once = "Testing, testing, one, two, three, testing, testing, you and me.";
        assert!(!looped(&format!("{once} {once}")), "said twice on purpose");
        assert!(looped(&format!("{once} {once} {once} Testing, testing, one")));
        assert!(!looped("go go go go go"), "short repeats are collapse_repeats' job");
        assert!(!looped("We met on Monday. Then we met on Tuesday. Then we met on Wednesday."));
    }

    /// Needs the model and a 16 kHz mono 16-bit WAV: set STV_TEST_WAV and run with --ignored.
    #[test]
    #[ignore]
    fn transcribes_a_real_file() {
        let data = std::fs::read(std::env::var("STV_TEST_WAV").unwrap()).unwrap();
        let at = data.windows(4).position(|w| w == b"data").unwrap() + 8;
        let pcm: Vec<i16> = data[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let text = Transcriber::load().unwrap().transcribe(&pcm, 0, pcm.len()).unwrap();
        println!("{text}");
        assert!(text.to_lowercase().contains(&std::env::var("STV_TEST_EXPECT").unwrap().to_lowercase()));
    }
}
