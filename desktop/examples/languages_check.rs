//! Checks languages end to end on this machine, in a scratch folder so the app's own data isn't touched
//! (apart from reading the Chinese voice and the language model already downloaded there):
//!
//!   cargo run --release --example languages_check -- <scratch folder> [speech model id] [language model file]
//!
//! 1. Says Chinese mixed with English, and Spanish, with natural voices, and transcribes them with the
//!    multilingual speech model (downloaded into the scratch folder), telling the languages apart.
//! 2. Writes a Welsh note, translates it into English, analyses it via English and translates the
//!    analysis back into Welsh.
use std::path::PathBuf;
use std::time::Instant;
use superteem_voice_lib::{analysis, library, llm, notes, transcribe, translations, tts};

/// 16 kHz mono 16-bit samples from a WAV, resampled if it's at another rate.
fn pcm_16k(wav: &[u8]) -> Vec<i16> {
    let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap()) as f64;
    let at = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
    let pcm: Vec<i16> = wav[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    if (rate - 16_000.0).abs() < 1.0 {
        return pcm;
    }
    let n = (pcm.len() as f64 * 16_000.0 / rate) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * rate / 16_000.0;
            let j = x as usize;
            let f = x - j as f64;
            let a = pcm[j.min(pcm.len() - 1)] as f64;
            let b = pcm[(j + 1).min(pcm.len() - 1)] as f64;
            (a + (b - a) * f) as i16
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scratch = PathBuf::from(&args[0]);
    let speech = args.get(1).cloned().unwrap_or_else(|| "small".into());
    let llm_file = args.get(2).cloned();

    // Before the scratch folder is set: the Chinese voice already downloaded in the app's own folder.
    let zh = tts::find_voice("kokoro-multi-lang-v1_1#58").unwrap();
    assert!(tts::ready(&zh), "the Kokoro voice isn't downloaded");
    let mixed = pcm_16k(&tts::speak_as(&zh, "我们下周发布新版本。这个 deadline 很重要，请大家准备好。", 1.0).unwrap());
    let english = pcm_16k(&tts::speak_as(&zh, "Please send the pricing sheet to Dana by Friday, and book the meeting room.", 1.0).unwrap());
    let model_dir = library::models_dir();

    library::init(scratch.join("sessions"), scratch.join("data"));
    std::fs::create_dir_all(scratch.join("data")).unwrap();

    // Spanish, with a small Spanish voice downloaded into the scratch folder.
    let es = tts::add(
        tts::NewVoice {
            name: "Sharvard".into(),
            language: "Spanish".into(),
            gender: String::new(),
            speaker: 0,
            url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-es_ES-sharvard-medium-int8.tar.bz2".into(),
            file: String::new(),
        },
        |_| {},
    )
    .unwrap();
    let spanish = pcm_16k(&tts::speak_as(&es, "Buenos días a todos. Hoy vamos a hablar del presupuesto del próximo trimestre.", 1.0).unwrap());

    let m = transcribe::model(&speech).unwrap();
    if !transcribe::is_ready(m) {
        println!("downloading the {} speech model…", m.name);
        transcribe::download(m.id, &Default::default(), |_| {}).unwrap();
    }
    let t = transcribe::Transcriber::load(m).unwrap();
    let pair = vec!["zh".to_string(), "en".to_string()];
    for (what, pcm, languages) in [("mixed", &mixed, &pair), ("english", &english, &pair), ("spanish", &spanish, &vec!["es".to_string()])] {
        let started = Instant::now();
        let language = t.detect(pcm, 0, pcm.len(), languages).unwrap_or_else(|| languages[0].clone());
        let text = t.transcribe(pcm, 0, pcm.len(), None, &language).unwrap();
        println!("{what}: heard as {language} in {:.1} s: {text}", started.elapsed().as_secs_f32());
    }

    // Translation and analysis via English, with the language model downloaded in the app's own folder.
    let Some(file) = llm_file else { return };
    let path = model_dir.join("llm").join(file);
    let device = llm::device();
    let mut model = llm::Local::load(&path, device.context, device.threads, true, Default::default()).unwrap();
    let engine = analysis::Engine { kind: "local".into(), model: model.name.clone(), host: None };
    let body = "Bore da bawb. Heddiw rydyn ni'n trafod y gyllideb ar gyfer y chwarter nesaf. \
        Bydd Siân yn anfon y daflen brisiau at Dafydd erbyn dydd Gwener. \
        Mae angen i ni benderfynu pryd i lansio'r fersiwn newydd. Fe wnawn ni lansio ym mis Mawrth.";
    let id = notes::save(None, "Cyfarfod y gyllideb".into(), body.into(), false).unwrap();
    notes::set_languages(&id, vec!["cy".into()]).unwrap();
    let item = analysis::Item::new("note", &id).unwrap();
    let started = Instant::now();
    translations::update(&item, "en", &mut model, &engine.model, |k, n| println!("translating {k}/{n}")).unwrap();
    println!("translated in {:.1} s:", started.elapsed().as_secs_f32());
    for l in translations::view(&item, "en").unwrap().lines {
        println!("  {} → {}{}", l.original, l.text, l.warning.map(|w| format!("  [{w}]")).unwrap_or_default());
    }
    let started = Instant::now();
    for e in analysis::make(&item, "meeting-summary@cy", &mut model, engine, |step, k, n| println!("{step} {k}/{n}")).unwrap() {
        println!("{} ({}, via {}): {}", e.template.name, if e.language.is_empty() { "en" } else { &e.language }, if e.via.is_empty() { "original" } else { &e.via }, e.result);
        analysis::put(&item, e).unwrap();
    }
    println!("analysed in {:.1} s", started.elapsed().as_secs_f32());
}
