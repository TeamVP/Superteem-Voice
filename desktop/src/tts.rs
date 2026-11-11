//! Natural voices for reading aloud, as an option: models from sherpa-onnx's collection
//! (https://k2-fsa.github.io/sherpa/onnx/tts/all/) run offline, each downloaded once when the person
//! chooses it. Without one, the system's own voice reads. Windows for now.
//!
//! A voice is a model and, for a model with many speakers, which one. The app offers some; the person can
//! add others from the collection, naming them as they like. Piper (VITS), Kokoro, Matcha and Kitten models
//! run; which one a folder holds is worked out from its files.

use crate::{library, prefs, transcribe};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const RELEASES: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Voice {
    /// What Settings stores: the model's name, then "#speaker" when it isn't the first.
    pub id: String,
    pub name: String,
    /// As shown: "British English", "Chinese English".
    pub language: String,
    /// "Female", "Male", or empty when not said.
    #[serde(default)]
    pub gender: String,
    /// What it can read, as language codes ("en", "zh").
    #[serde(default)]
    pub langs: Vec<String>,
    #[serde(default)]
    pub recommended: bool,
    /// The model's folder, which is also its archive's name in the collection.
    pub model: String,
    /// Which of a model's speakers this is.
    #[serde(default)]
    pub speaker: i32,
    /// Added by the person.
    #[serde(default)]
    pub custom: bool,
    /// The download, in bytes; 0 for one added by the person.
    #[serde(default)]
    pub size: u64,
}

/// A model the app offers, pinned to its file.
struct Model {
    name: &'static str,
    size: u64,
    sha256: &'static str,
}

const MODELS: &[Model] = &[
    Model { name: "vits-piper-en_GB-alba-medium", size: 67_212_349, sha256: "fcd45962906933eec4431d3688f7d74aaac8713c87c6717f91fd3b23463aa1a1" },
    Model { name: "vits-piper-en_GB-alan-medium", size: 67_220_121, sha256: "a48d4017da0f77668b27bed63fe6e04dd64c6397e1fadad4f460efb0ef7c9012" },
    Model { name: "vits-piper-en_GB-cori-medium", size: 67_257_412, sha256: "49c9a5361bbdd95d7ca9687c4de11e5908481f65e7c7c368960df79949fdac2b" },
    Model { name: "vits-piper-en_GB-jenny_dioco-medium", size: 67_225_842, sha256: "a0888024569bafbefc05a4b48ddf8419d8dbbf3205f4af37cf7c6f1a87cc20c5" },
    Model { name: "vits-piper-en_GB-northern_english_male-medium", size: 67_210_490, sha256: "2bb2c1e709f58c11f17c693b3b38f500e110e7f54f2651774ec48b8d41f12c55" },
    Model { name: "vits-piper-en_US-amy-medium", size: 67_223_746, sha256: "9a5d1fc497f85e8022b785bff5f8105203b1e33099ee6265203efc70b0cb0264" },
    Model { name: "vits-piper-en_US-hfc_male-medium", size: 67_214_049, sha256: "76388f84acfca8ba5c0ed1636a26ada14c598abd52e76f110d4756fe326fc5f2" },
    Model { name: "vits-piper-en_US-joe-medium", size: 67_169_394, sha256: "261b903a1fb2ce621e2b5b14233cd41223c3b9f9590f896c11daee4df22d9bd3" },
    Model { name: "vits-piper-en_US-lessac-medium", size: 67_230_653, sha256: "9e3febfacf0abf4270172d2958bcec246032b7e88efc2720840cc80c93de334e" },
    Model { name: "vits-piper-en_US-norman-medium", size: 67_203_672, sha256: "1f32065d480abe9abc7c7f91442125d0b34c1cc065d1e600466cac408eabf3b8" },
    Model { name: "vits-piper-en_US-ryan-medium", size: 67_213_100, sha256: "c546af78b6395b4e7c4ce1ed899438b64426a362f5d4ec5fecd090ded9ad7505" },
    Model { name: "kokoro-multi-lang-v1_1", size: 364_855_331, sha256: "b78e1ac42b852e839cceaafbe88f66806bf5860992d1d7b9a3f2ad1198427aed" },
    Model { name: "matcha-icefall-zh-en", size: 79_033_838, sha256: "271b804af570400d3bcdcb53bf6e53cc9f75180ee763b9f13eb5eaf2b0d086ef" },
    // A voice for each language Voice offers where the collection has one, so translations can be heard.
    Model { name: "vits-piper-es_ES-davefx-medium", size: 67_184_952, sha256: "a3f6beb54a9cb893279f72978a22f807a4d9fc9c7848157b524d5cc7b7f58b22" },
    Model { name: "vits-piper-es_AR-daniela-high", size: 115_562_134, sha256: "71cbf6b7f646ab74f3c51336151abef41e6c54467ac929ffdb19ed706a07dd7b" },
    Model { name: "vits-piper-es_MX-ald-medium", size: 67_196_497, sha256: "c60cf7bc853eb01a875fba0756523fecf9d9f46c763b19785d223fc981fb4fb6" },
    Model { name: "vits-piper-fr_FR-siwis-medium", size: 67_207_459, sha256: "375909aa30842b3a4efa10b1beb1d761af792960ae6873b4d53889f96c66195b" },
    Model { name: "vits-piper-fr_FR-tom-medium", size: 67_202_955, sha256: "3d8258ef8466b5d5f2cc46c06e1415d3d01c9cbaeed8b6d5b3d152073672472c" },
    Model { name: "vits-piper-de_DE-thorsten-medium", size: 67_214_254, sha256: "50487d9c95fdf2191f31d2588569381063ba1591dcd4c7d4bdd30f12b2191714" },
    Model { name: "vits-piper-de_DE-kerstin-low", size: 67_107_768, sha256: "4bfe121d5f690a3b87acecd7ae1e70b48f4589ce6ae9f45a1809e6eb19fad97b" },
    Model { name: "vits-piper-it_IT-paola-medium", size: 67_221_173, sha256: "7541f75778afa164e44e34baaef63befad7698595df26a95ca944b63ef1a16b4" },
    Model { name: "vits-piper-it_IT-riccardo-x_low", size: 26_496_614, sha256: "8ad0791558ae2b3e96054fc5756ae3ae2f6457fb1765bc01f43bbab85e0a7021" },
    Model { name: "vits-piper-pt_BR-faber-medium", size: 67_183_065, sha256: "7add3f923ad6bc25ca8a192805fd1a64d1b3893e4611c4a9719545a825039a83" },
    Model { name: "vits-piper-pt_PT-tugao-medium", size: 67_195_014, sha256: "6b4f0a7bfc957bef26d6ac0703a8fa1ba659c8ab2df8b294f20206551859070c" },
    Model { name: "vits-piper-nl_NL-pim-medium", size: 67_240_538, sha256: "e78ed301bf6fc66561a04c110754cd6928d772630a3b80b09a83f89410130171" },
    Model { name: "vits-piper-nl_BE-nathalie-medium", size: 67_226_472, sha256: "c7484313abea395243e872196bc75181c07aad8120b8a61e0f6f4b3863b6a7c8" },
    Model { name: "vits-piper-pl_PL-gosia-medium", size: 67_211_182, sha256: "75bd34dcbdc4dd98d763954756b4b34b4208100497c836381542e4d73dcefa9c" },
    Model { name: "vits-piper-pl_PL-darkman-medium", size: 67_215_635, sha256: "444727aa46eb6db645a2bc88fe73868e4bd7596b7f56ca39fad5ef53c41210d4" },
    Model { name: "vits-piper-ru_RU-irina-medium", size: 67_153_308, sha256: "1fc0f54e5e084fe287c07909f2f6e0ba6d857864cf800e3ab80286a4e8233008" },
    Model { name: "vits-piper-ru_RU-dmitri-medium", size: 67_188_551, sha256: "c86d0803737de13d441923ff3b3f309482fab8d7af3ec85949942809eb9a3660" },
    Model { name: "vits-piper-uk_UA-ukrainian_tts-medium", size: 80_355_083, sha256: "92be6ff21ebb1cbcc0c71121a6437db8d060cee5785287aaa6a19a2548f59c03" },
    Model { name: "vits-piper-sv_SE-lisa-medium", size: 67_223_955, sha256: "78b5f6808e513d545d9284a015075ef31f09623e6c08d6186a29a1997aa577d7" },
    Model { name: "vits-piper-da_DK-talesyntese-medium", size: 67_185_257, sha256: "f2e2cbcdb2b21e76f93635b35105d07b37997ace434d7289d04b689a97c19bd0" },
    Model { name: "vits-piper-no_NO-talesyntese-medium", size: 67_194_829, sha256: "dcb7c36b7222e290cca1fd4cdecb474e7fd651dc5b0d3d42c33d416a8e62018b" },
    Model { name: "vits-piper-fi_FI-harri-medium", size: 67_205_198, sha256: "a7fd608e73983c366a5e0bcd13382e6529a6749f67efc406b5a7f9021e3a8b1d" },
    Model { name: "vits-piper-is_IS-ugla-medium", size: 67_376_655, sha256: "2d2295b13746758620ef02ee35294ff1096d94fd01cb055c53f70de17a222519" },
    Model { name: "vits-piper-cs_CZ-jirka-medium", size: 67_212_775, sha256: "321aafd83dd89b3d5b2c36c5898e0710d4f472f3c191cb7f73901a89b7f8539f" },
    Model { name: "vits-piper-sk_SK-lili-medium", size: 67_183_092, sha256: "a00dfbca5d34733862a59d9e309f4e94a92fccc9337c2b124de9a241b604fc0d" },
    Model { name: "vits-piper-sl_SI-artur-medium", size: 67_025_668, sha256: "df805f31c4513864edebf7e287cf0b54d72788fc661ed73f88dc4e5aabc5efd5" },
    Model { name: "vits-piper-hu_HU-anna-medium", size: 67_167_701, sha256: "c67b13e57b71eca19098fe499afc35ebce8f63a0b524017c786af2025821216a" },
    Model { name: "vits-piper-ro_RO-mihai-medium", size: 67_182_137, sha256: "cfbee3cdc25cf063339f69e189f4d93cc3be6610d267567537323ced3cf87f4d" },
    Model { name: "vits-piper-sr_RS-serbski_institut-medium", size: 80_298_557, sha256: "b8c100e5ff885d11ff96ad92a953f4b82e1cd649f9ac60a77ae4553d37507b2b" },
    Model { name: "vits-piper-lv_LV-aivars-medium", size: 67_196_173, sha256: "a0df58c9924761e201ef75d82473b90bde32c5372267e9b4c628fdd267cf2508" },
    Model { name: "vits-piper-el_GR-rapunzelina-low", size: 67_105_521, sha256: "bb868070a9ba0d7220b3d7c9291a998c2f60accdb73525bd63b8751b86dd4839" },
    Model { name: "vits-piper-tr_TR-fettah-medium", size: 67_174_342, sha256: "314a9910616fb17be882c77f0bcf76796a1914d4d606f3c984f9094cb9abf1e5" },
    Model { name: "vits-piper-ar_JO-kareem-medium", size: 67_177_830, sha256: "9ebbcea30e0fbd588f7b2cb45ee897d6aeb1bf5791cbc037a7b5a3f641e3dbce" },
    Model { name: "vits-piper-fa_IR-amir-medium", size: 67_180_373, sha256: "46af65c1cdd86ec300db3998f2bf31a68d9e38da930c0655c1ff6f610e84f864" },
    Model { name: "vits-piper-ur_PK-fasih-medium", size: 67_227_718, sha256: "de64e03109f3c896b7cf910db6d9ffd2703e3d665580d9a19bedbf3e0760d0e4" },
    Model { name: "vits-piper-hi_IN-priyamvada-medium", size: 67_240_610, sha256: "399d91cc97eb288725633261f26b715f9a971e3bf7ec4fa1d7910cd0080d37eb" },
    Model { name: "vits-piper-hi_IN-pratham-medium", size: 67_238_438, sha256: "2084d321e1d2752f2b64ed3012ba27751df01a80da46f52920098cdcb7e35648" },
    Model { name: "vits-piper-ne_NP-google-medium", size: 80_242_801, sha256: "73c1e093451867779ec2b55cc3666de2db058019a2e7760e8c8f294a42caca2a" },
    Model { name: "vits-piper-ml_IN-meera-medium", size: 67_195_010, sha256: "c8d870441e60870b7e8781c26eb22104aff51587b63bee94aba5e5f6c9fbb71f" },
    Model { name: "vits-piper-vi_VN-vais1000-medium", size: 67_154_040, sha256: "fa1367710767d36ed5cf13b4a449e20c35ffd12791c2e47c2e64142bfa55551a" },
    Model { name: "vits-piper-id_ID-news_tts-medium", size: 67_243_848, sha256: "634b0169fe41c368e9fafe128f30647f9e0301e852ce503cd78146db524f1f12" },
    Model { name: "vits-piper-ka_GE-natia-medium", size: 67_209_457, sha256: "18e34c601f4be13e5132ec772e2cbcfa47ca6a4aaaf995dd5baefe014f5a0256" },
    Model { name: "vits-piper-kk_KZ-raya-x_low", size: 26_497_880, sha256: "fce067c8c078d96586fba3a429b1acbf7911a1248b04eebfebab2cf110966ce0" },
    Model { name: "vits-piper-ca_ES-upc_ona-medium", size: 67_223_188, sha256: "b6c50f22e13cfc72ce51c496d22b987adf6704047f6d342968197b4b0b3127a5" },
    Model { name: "vits-piper-eu_ES-maider-medium", size: 67_247_291, sha256: "60493b81a7f117ea96b6efc841d7465c1d4107ea72ef553ff91f92f72fbd95c1" },
    Model { name: "vits-piper-cy_GB-gwryw_gogleddol-medium", size: 67_230_234, sha256: "f31f90afc4f28e9a768309f0b1c87272ef178936b32dd181aa1aa3a0eeddd187" },
    Model { name: "vits-piper-sw_CD-lanfrica-medium", size: 67_217_230, sha256: "93196704a0ae0c984806059480775d69dcc2419d0d545cddd0903d244dc66a72" },
];

/// Matcha models make sound through a separate vocoder, shared between them.
struct Vocoder {
    file: &'static str,
    size: u64,
    /// Empty where the collection doesn't publish one.
    sha256: &'static str,
}

const VOCODER_16K: Vocoder = Vocoder { file: "vocos-16khz-univ.onnx", size: 53_882_848, sha256: "b599142a1fb8ff03de3e84ac35ff537c619e56f4267a6fe894851a42844acf9e" };
const VOCODER_22K: Vocoder = Vocoder { file: "vocos-22khz-univ.onnx", size: 53_884_024, sha256: "" };

/// The Chinese and English Matcha model was trained with the 16 kHz vocoder; the others with the 22 kHz one.
fn vocoder_for(model: &str) -> &'static Vocoder {
    if model.contains("zh-en") { &VOCODER_16K } else { &VOCODER_22K }
}

#[allow(clippy::too_many_arguments)]
fn voice(id: &str, name: &str, language: &str, gender: &str, langs: &[&str], recommended: bool, model: &str, speaker: i32) -> Voice {
    Voice {
        id: id.into(),
        name: name.into(),
        language: language.into(),
        gender: gender.into(),
        langs: langs.iter().map(|s| s.to_string()).collect(),
        recommended,
        model: model.into(),
        speaker,
        custom: false,
        size: MODELS.iter().find(|m| m.name == model).map_or(0, |m| m.size + if model.starts_with("matcha") { vocoder_for(model).size } else { 0 }),
    }
}

/// The voices the app offers.
fn built_in() -> Vec<Voice> {
    let piper = |name: &str, language: &str, gender: &str, recommended: bool, model: &str| voice(model, name, language, gender, &["en"], recommended, model, 0);
    let other = |name: &str, language: &str, gender: &str, code: &str, model: &str| voice(model, name, language, gender, &[code], false, model, 0);
    vec![
        piper("Alba", "Scottish English", "Female", true, "vits-piper-en_GB-alba-medium"),
        piper("Joe", "US English", "Male", true, "vits-piper-en_US-joe-medium"),
        piper("Alan", "British English", "Male", false, "vits-piper-en_GB-alan-medium"),
        piper("Cori", "British English", "Female", false, "vits-piper-en_GB-cori-medium"),
        piper("Jenny", "British English", "Female", false, "vits-piper-en_GB-jenny_dioco-medium"),
        piper("Northern English", "British English", "Male", false, "vits-piper-en_GB-northern_english_male-medium"),
        piper("Amy", "US English", "Female", false, "vits-piper-en_US-amy-medium"),
        piper("Henrik", "US English", "Male", false, "vits-piper-en_US-hfc_male-medium"),
        piper("Lessac", "US English", "Female", false, "vits-piper-en_US-lessac-medium"),
        piper("Norman", "US English", "Male", false, "vits-piper-en_US-norman-medium"),
        piper("Ryan", "US English", "Male", false, "vits-piper-en_US-ryan-medium"),
        // Speaker 58 is zm_009.
        voice("kokoro-multi-lang-v1_1#58", "Yǔxuān (宇轩)", "Chinese English", "Male", &["zh", "en"], false, "kokoro-multi-lang-v1_1", 58),
        voice("matcha-icefall-zh-en", "Chá (茶)", "Chinese English", "Female", &["zh", "en"], false, "matcha-icefall-zh-en", 0),
        // Other languages, each language's first voice the one offered for it. Gender is left out where
        // the collection doesn't say.
        other("Dave", "Spanish (Spain)", "Male", "es", "vits-piper-es_ES-davefx-medium"),
        other("Daniela", "Spanish (Argentina)", "Female", "es", "vits-piper-es_AR-daniela-high"),
        other("Aldo", "Spanish (Mexico)", "Male", "es", "vits-piper-es_MX-ald-medium"),
        other("Siwis", "French", "Female", "fr", "vits-piper-fr_FR-siwis-medium"),
        other("Tom", "French", "Male", "fr", "vits-piper-fr_FR-tom-medium"),
        other("Thorsten", "German", "Male", "de", "vits-piper-de_DE-thorsten-medium"),
        other("Kerstin", "German", "Female", "de", "vits-piper-de_DE-kerstin-low"),
        other("Paola", "Italian", "Female", "it", "vits-piper-it_IT-paola-medium"),
        other("Riccardo", "Italian", "Male", "it", "vits-piper-it_IT-riccardo-x_low"),
        other("Faber", "Portuguese (Brazil)", "Male", "pt", "vits-piper-pt_BR-faber-medium"),
        other("Tugão", "Portuguese (Portugal)", "Male", "pt", "vits-piper-pt_PT-tugao-medium"),
        other("Pim", "Dutch", "Male", "nl", "vits-piper-nl_NL-pim-medium"),
        other("Nathalie", "Dutch (Belgium)", "Female", "nl", "vits-piper-nl_BE-nathalie-medium"),
        other("Gosia", "Polish", "Female", "pl", "vits-piper-pl_PL-gosia-medium"),
        other("Darkman", "Polish", "Male", "pl", "vits-piper-pl_PL-darkman-medium"),
        other("Irina", "Russian", "Female", "ru", "vits-piper-ru_RU-irina-medium"),
        other("Dmitri", "Russian", "Male", "ru", "vits-piper-ru_RU-dmitri-medium"),
        other("Ukrainian TTS", "Ukrainian", "", "uk", "vits-piper-uk_UA-ukrainian_tts-medium"),
        other("Lisa", "Swedish", "Female", "sv", "vits-piper-sv_SE-lisa-medium"),
        other("Talesyntese", "Danish", "Male", "da", "vits-piper-da_DK-talesyntese-medium"),
        other("Talesyntese", "Norwegian", "Male", "no", "vits-piper-no_NO-talesyntese-medium"),
        other("Harri", "Finnish", "Male", "fi", "vits-piper-fi_FI-harri-medium"),
        other("Ugla", "Icelandic", "Female", "is", "vits-piper-is_IS-ugla-medium"),
        other("Jirka", "Czech", "Male", "cs", "vits-piper-cs_CZ-jirka-medium"),
        other("Lili", "Slovak", "Female", "sk", "vits-piper-sk_SK-lili-medium"),
        other("Artur", "Slovenian", "Male", "sl", "vits-piper-sl_SI-artur-medium"),
        other("Anna", "Hungarian", "Female", "hu", "vits-piper-hu_HU-anna-medium"),
        other("Mihai", "Romanian", "Male", "ro", "vits-piper-ro_RO-mihai-medium"),
        other("Serbski Institut", "Serbian", "", "sr", "vits-piper-sr_RS-serbski_institut-medium"),
        other("Aivars", "Latvian", "Male", "lv", "vits-piper-lv_LV-aivars-medium"),
        other("Rapunzelina", "Greek", "Female", "el", "vits-piper-el_GR-rapunzelina-low"),
        other("Fettah", "Turkish", "Male", "tr", "vits-piper-tr_TR-fettah-medium"),
        other("Kareem", "Arabic", "Male", "ar", "vits-piper-ar_JO-kareem-medium"),
        other("Amir", "Persian", "Male", "fa", "vits-piper-fa_IR-amir-medium"),
        other("Fasih", "Urdu", "Male", "ur", "vits-piper-ur_PK-fasih-medium"),
        other("Priyamvada", "Hindi", "Female", "hi", "vits-piper-hi_IN-priyamvada-medium"),
        other("Pratham", "Hindi", "Male", "hi", "vits-piper-hi_IN-pratham-medium"),
        other("Google", "Nepali", "", "ne", "vits-piper-ne_NP-google-medium"),
        other("Meera", "Malayalam", "Female", "ml", "vits-piper-ml_IN-meera-medium"),
        other("Vais1000", "Vietnamese", "", "vi", "vits-piper-vi_VN-vais1000-medium"),
        other("News", "Indonesian", "", "id", "vits-piper-id_ID-news_tts-medium"),
        other("Natia", "Georgian", "Female", "ka", "vits-piper-ka_GE-natia-medium"),
        other("Raya", "Kazakh", "", "kk", "vits-piper-kk_KZ-raya-x_low"),
        other("Ona", "Catalan", "Female", "ca", "vits-piper-ca_ES-upc_ona-medium"),
        other("Maider", "Basque", "Female", "eu", "vits-piper-eu_ES-maider-medium"),
        other("Gwryw Gogleddol", "Welsh", "Male", "cy", "vits-piper-cy_GB-gwryw_gogleddol-medium"),
        other("Lanfrica", "Swahili", "", "sw", "vits-piper-sw_CD-lanfrica-medium"),
    ]
}

/// Recommended first, then by language (British English before US English), then female before male,
/// then by name.
fn sort(voices: &mut [Voice]) {
    let gender = |g: &str| match g {
        "Female" => 0,
        "Male" => 1,
        _ => 2,
    };
    voices.sort_by(|a, b| {
        (!a.recommended, a.language.to_lowercase(), gender(&a.gender), a.name.to_lowercase())
            .cmp(&(!b.recommended, b.language.to_lowercase(), gender(&b.gender), b.name.to_lowercase()))
    });
}

fn root() -> PathBuf {
    library::models_dir().join("tts")
}

fn custom_file() -> PathBuf {
    root().join("custom.json")
}

fn custom() -> Vec<Voice> {
    library::read_json(&custom_file()).unwrap_or_default()
}

/// The voice offered first for a language: the catalogue's own order, which puts the best of each first.
pub fn first_for(language: &str) -> Option<Voice> {
    built_in().into_iter().find(|v| v.langs.iter().any(|l| l == language))
}

/// The voices the app offers, in the order they're listed, without the person's own.
pub fn offered() -> Vec<Voice> {
    let mut all = built_in();
    sort(&mut all);
    all
}

/// Every voice, in the order they're listed.
pub fn voices() -> Vec<Voice> {
    let mut all = built_in();
    all.extend(custom());
    sort(&mut all);
    all
}

pub fn supported() -> bool {
    cfg!(windows)
}

fn find(id: &str) -> Result<Voice, String> {
    voices().into_iter().find(|v| v.id == id).ok_or_else(|| "That voice isn't in the list.".into())
}

/// The voice chosen in Settings; when none is, the first downloaded, so a voice fetched before there was a
/// choice keeps reading; else the first in the list.
pub fn chosen_in(p: &prefs::Prefs, all: &[Voice]) -> Voice {
    all.iter().find(|v| v.id == p.tts_voice).or_else(|| all.iter().find(|v| ready(v))).unwrap_or(&all[0]).clone()
}

/// The natural voice that reads `language` (a code), or None for the system's voice. A voice set for the
/// language comes first; then the main voice, when it speaks the language; then the first downloaded voice
/// that does. With natural voices off, the system's voice reads the primary language, and a downloaded voice
/// any other it speaks.
pub fn for_language(p: &prefs::Prefs, language: Option<&str>) -> Option<Voice> {
    let all = voices();
    if let Some(lang) = language {
        match p.voice_for.get(lang).map(String::as_str) {
            Some("system") => return None,
            Some(id) => {
                if let Some(v) = all.iter().find(|v| v.id == id && ready(v)) {
                    return Some(v.clone());
                }
            }
            None => {}
        }
    }
    let speaks = |v: &Voice, lang: &str| v.langs.iter().any(|l| l == lang);
    if !p.natural_voice {
        // The system's voice reads the person's own language. A voice downloaded for another, such as one
        // a translation is in, reads that one: the system rarely has a voice for it.
        return match language {
            Some(lang) if lang != p.primary_language() => all.into_iter().find(|v| speaks(v, lang) && ready(v)),
            _ => None,
        };
    }
    let main = chosen_in(p, &all);
    match language {
        None => ready(&main).then_some(main),
        Some(lang) if speaks(&main, lang) && ready(&main) => Some(main),
        Some(lang) => all.into_iter().find(|v| speaks(v, lang) && ready(v)),
    }
}

pub fn chosen() -> Voice {
    chosen_in(&prefs::load(), &voices())
}

/// What kind of model a folder holds, from its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Vits,
    Kokoro,
    Kitten,
    Matcha,
}

fn files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    out.sort();
    out
}

fn kind(dir: &Path) -> Option<Kind> {
    let names = files(dir);
    let has = |n: &str| names.iter().any(|f| f == n);
    let folder = dir.file_name()?.to_string_lossy().to_lowercase();
    if !has("tokens.txt") {
        return None;
    }
    if has("voices.bin") {
        return Some(if folder.contains("kitten") { Kind::Kitten } else { Kind::Kokoro });
    }
    if folder.starts_with("matcha") || names.iter().any(|f| f.starts_with("model-steps") && f.ends_with(".onnx")) {
        return Some(Kind::Matcha);
    }
    names.iter().any(|f| f.ends_with(".onnx")).then_some(Kind::Vits)
}

fn model_dir(v: &Voice) -> PathBuf {
    root().join(&v.model)
}

fn vocoder_path(model: &str) -> PathBuf {
    root().join("vocoders").join(vocoder_for(model).file)
}

pub fn ready(v: &Voice) -> bool {
    match kind(&model_dir(v)) {
        Some(Kind::Matcha) => vocoder_path(&v.model).is_file(),
        Some(_) => true,
        None => false,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceStatus {
    #[serde(flatten)]
    pub voice: Voice,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub supported: bool,
    pub voices: Vec<VoiceStatus>,
    /// The natural voice that reads when natural voices are on.
    pub chosen: String,
    /// The model being downloaded, if one is.
    pub downloading: Option<String>,
}

static DOWNLOADING: Mutex<Option<String>> = Mutex::new(None);
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn status() -> Status {
    let all = voices();
    Status {
        supported: supported(),
        chosen: chosen_in(&prefs::load(), &all).id,
        voices: all.into_iter().map(|v| VoiceStatus { ready: ready(&v), voice: v }).collect(),
        downloading: DOWNLOADING.lock().unwrap().clone(),
    }
}

pub fn cancel_download() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Deletes a voice's download, and takes a voice the person added out of the list. A model another voice
/// the person added still uses stays.
pub fn delete(id: &str) -> Result<(), String> {
    let v = find(id)?;
    unload();
    if v.custom {
        let rest: Vec<Voice> = custom().into_iter().filter(|c| c.id != v.id).collect();
        library::write_json(&custom_file(), &rest)?;
    }
    let still_used = custom().iter().any(|o| o.id != v.id && o.model == v.model);
    if !still_used {
        let _ = std::fs::remove_dir_all(model_dir(&v));
    }
    Ok(())
}

/// Runs `f` as the one download at a time, named by `what`.
fn one_download<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    {
        let mut busy = DOWNLOADING.lock().unwrap();
        if busy.is_some() {
            return Err("Another voice is downloading. Wait for it to finish.".into());
        }
        *busy = Some(what.to_string());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let result = f();
    *DOWNLOADING.lock().unwrap() = None;
    result
}

/// Downloads `url` to `to`, reporting the fraction done of `size`, checking `sha256` when one is given.
fn fetch(url: &str, to: &Path, size: u64, sha256: &str, progress: &dyn Fn(f32)) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let response = transcribe::agent()?.get(url).call().map_err(|e| format!("The download didn't start: {e}"))?;
    let size = if size > 0 { size } else { response.header("Content-Length").and_then(|l| l.parse().ok()).unwrap_or(0) };
    let mut reader = response.into_reader();
    let part = to.with_extension("part");
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let (mut have, mut buffer) = (0u64, vec![0u8; 1 << 16]);
    loop {
        if CANCEL.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err("Download stopped.".into());
        }
        let n = reader.read(&mut buffer).map_err(|e| format!("The download was interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        have += n as u64;
        if size > 0 {
            progress((have as f32 / size as f32).min(1.0));
        }
    }
    drop(file);
    let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !sha256.is_empty() && hash != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err("The download didn't match, so it was deleted. Try again.".into());
    }
    std::fs::rename(&part, to).map_err(|e| e.to_string())
}

/// Unpacks a model archive into the voices folder, returning the folder it made.
fn unpack(archive: &Path) -> Result<String, String> {
    let open = || std::fs::File::open(archive).map_err(|e| e.to_string());
    // Its first entry names its folder.
    let mut listing = tar::Archive::new(bzip2::read::BzDecoder::new(open()?));
    let first = listing.entries().map_err(|e| format!("That isn't a model archive: {e}"))?.next().ok_or("The archive is empty.")?;
    let path = first.map_err(|e| e.to_string())?.path().map_err(|e| e.to_string())?.into_owned();
    let folder = path.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).ok_or("The archive is empty.")?;
    if folder.contains(['/', '\\']) || folder.starts_with('.') || folder.is_empty() || folder == "vocoders" {
        return Err("That isn't a model archive.".into());
    }
    let _ = std::fs::remove_dir_all(root().join(&folder));
    // Entry by entry: one that would land outside the voices folder is skipped, not followed.
    let mut archive = tar::Archive::new(bzip2::read::BzDecoder::new(open()?));
    for entry in archive.entries().map_err(|e| format!("Couldn't unpack it: {e}"))? {
        let mut entry = entry.map_err(|e| format!("Couldn't unpack it: {e}"))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        let relative = entry.path().map_err(|e| e.to_string())?.into_owned();
        if !relative.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
            continue;
        }
        let to = root().join(&relative);
        if kind.is_dir() {
            std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        entry.unpack(&to).map_err(|e| format!("Couldn't unpack it: {e}"))?;
    }
    Ok(folder)
}

/// The vocoder a Matcha model needs, fetched when it isn't here.
fn ensure_vocoder(model: &str, progress: &dyn Fn(f32)) -> Result<(), String> {
    let path = vocoder_path(model);
    if path.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().expect("has a folder")).map_err(|e| e.to_string())?;
    let v = vocoder_for(model);
    fetch(&format!("{RELEASES}vocoder-models/{}", v.file), &path, v.size, v.sha256, progress)
}

/// Downloads an offered voice's model, one at a time.
pub fn download(id: &str, progress: impl Fn(f32)) -> Result<(), String> {
    let v = find(id)?;
    let model = MODELS.iter().find(|m| m.name == v.model).ok_or("That voice was added by you; add it again to download it.")?;
    one_download(&v.model, || {
        std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
        let total = v.size.max(1) as f32;
        let share = model.size as f32 / total;
        let archive = root().join(format!("{}.tar.bz2", model.name));
        fetch(&format!("{RELEASES}tts-models/{}.tar.bz2", model.name), &archive, model.size, model.sha256, &|f| progress(f * share * 0.97))?;
        let unpacked = unpack(&archive);
        let _ = std::fs::remove_file(&archive);
        unpacked?;
        if kind(&model_dir(&v)) == Some(Kind::Matcha) {
            ensure_vocoder(&v.model, &|f| progress(share + f * (1.0 - share)))?;
        }
        progress(1.0);
        if ready(&v) { Ok(()) } else { Err("The voice was unpacked but isn't complete.".into()) }
    })
}

/// What the person says about a voice they add.
#[derive(Debug, Clone, Deserialize)]
pub struct NewVoice {
    pub name: String,
    pub language: String,
    #[serde(default)]
    pub gender: String,
    #[serde(default)]
    pub speaker: i32,
    /// The model's download address from the collection ...
    #[serde(default)]
    pub url: String,
    /// ... or an archive on this computer.
    #[serde(default)]
    pub file: String,
}

/// Language codes a language's name suggests, for voices the person adds.
fn langs_of(language: &str) -> Vec<String> {
    let l = language.to_lowercase();
    let known = [
        ("arabic", "ar"),
        ("english", "en"),
        ("spanish", "es"),
        ("french", "fr"),
        ("hindi", "hi"),
        ("turkish", "tr"),
        ("welsh", "cy"),
        ("chinese", "zh"),
        ("mandarin", "zh"),
    ];
    let mut out: Vec<String> = known.iter().filter(|(word, _)| l.contains(word)).map(|(_, code)| code.to_string()).collect();
    out.sort();
    out.dedup();
    out
}

/// Adds a voice from the collection: downloads or unpacks its model, checks it speaks, and lists it.
pub fn add(new: NewVoice, progress: impl Fn(f32)) -> Result<Voice, String> {
    let name: String = new.name.trim().chars().take(60).collect();
    if name.is_empty() {
        return Err("Give the voice a name.".into());
    }
    if new.speaker < 0 {
        return Err("The speaker is a number from 0.".into());
    }
    let url = new.url.trim();
    let what = if url.is_empty() {
        Path::new(&new.file).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        url.rsplit('/').next().unwrap_or(url).to_string()
    };
    let folder = one_download(&what, || {
        std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
        let folder = if !url.is_empty() {
            if !(url.starts_with(&format!("{RELEASES}tts-models/")) && url.ends_with(".tar.bz2")) {
                return Err("Use the model's download address from its page in the sherpa-onnx collection: it starts https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/ and ends .tar.bz2.".into());
            }
            let archive = root().join("adding.tar.bz2");
            fetch(url, &archive, 0, "", &|f| progress(f * 0.9))?;
            let unpacked = unpack(&archive);
            let _ = std::fs::remove_file(&archive);
            unpacked?
        } else if new.file.to_lowercase().ends_with(".tar.bz2") {
            progress(0.3);
            unpack(Path::new(&new.file))?
        } else {
            return Err("Choose the model's .tar.bz2 file, or paste its download address.".into());
        };
        let dir = root().join(&folder);
        match kind(&dir) {
            Some(Kind::Matcha) => ensure_vocoder(&folder, &|f| progress(0.9 + f * 0.05))?,
            Some(_) => {}
            None => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err("That model isn't one Voice can run: it needs to be a Piper, Kokoro, Matcha or Kitten model.".into());
            }
        }
        Ok(folder)
    })?;
    let voice = Voice {
        id: if new.speaker == 0 { folder.clone() } else { format!("{folder}#{}", new.speaker) },
        name,
        language: new.language.trim().chars().take(40).collect(),
        gender: new.gender,
        langs: langs_of(&new.language),
        recommended: false,
        model: folder,
        speaker: new.speaker,
        custom: true,
        size: 0,
    };
    if built_in().iter().any(|b| b.id == voice.id) {
        return Err(format!("{} is already in the list.", voice.model));
    }
    // It has to speak before it's listed.
    if let Err(e) = imp::render(&voice, "Hello.", 1.0) {
        unload();
        if !custom().iter().any(|o| o.model == voice.model) {
            let _ = std::fs::remove_dir_all(model_dir(&voice));
        }
        return Err(format!("The model was downloaded but didn't speak: {e}"));
    }
    let mut list: Vec<Voice> = custom().into_iter().filter(|c| c.id != voice.id).collect();
    list.push(voice.clone());
    library::write_json(&custom_file(), &list)?;
    progress(1.0);
    Ok(voice)
}

/// Speech for `text` in the chosen voice at `speed` (1 is normal), as a 16-bit mono WAV.
pub fn speak(text: &str, speed: f32) -> Result<Vec<u8>, String> {
    speak_as(&chosen(), text, speed)
}

/// Speech for `text` in voice `v`.
pub fn speak_as(v: &Voice, text: &str, speed: f32) -> Result<Vec<u8>, String> {
    if !ready(v) {
        return Err(format!("The {} voice isn't downloaded. Choose one in Settings › Reading aloud.", v.name));
    }
    let (samples, rate) = imp::render(v, text, speed)?;
    Ok(wav(&samples, rate))
}

pub fn find_voice(id: &str) -> Result<Voice, String> {
    find(id)
}

fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let pcm: Vec<i16> = samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
    let bytes = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub use imp::unload;

#[cfg(windows)]
mod imp {
    use super::*;
    use sherpa_onnx::{
        GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKittenModelConfig, OfflineTtsKokoroModelConfig, OfflineTtsMatchaModelConfig,
        OfflineTtsModelConfig, OfflineTtsVitsModelConfig,
    };

    /// The loaded model, by folder.
    static ENGINE: Mutex<Option<(String, OfflineTts)>> = Mutex::new(None);

    /// sherpa-onnx is loaded on first use (see build.rs), so it's looked for first: calling into a DLL that
    /// isn't there would end the app.
    fn engine_present() -> bool {
        #[link(name = "kernel32")]
        extern "system" {
            fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
        }
        let name: Vec<u16> = "sherpa-onnx-c-api.dll".encode_utf16().chain(std::iter::once(0)).collect();
        !unsafe { LoadLibraryW(name.as_ptr()) }.is_null()
    }

    /// The settings for voice `v`'s model, from the files in its folder.
    fn config(v: &Voice) -> Result<OfflineTtsConfig, String> {
        let dir = model_dir(v);
        let names = files(&dir);
        let path = |f: &str| Some(dir.join(f).to_string_lossy().into_owned());
        let some = |f: &str| if names.iter().any(|n| n == f) { path(f) } else { None };
        let first = |test: &dyn Fn(&str) -> bool| names.iter().find(|n| test(n)).and_then(|n| path(n));
        let join = |list: Vec<&String>| (!list.is_empty()).then(|| list.iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect::<Vec<_>>().join(","));
        let onnx = |n: &str| n.ends_with(".onnx");
        // Number, date and phone rules for reading Chinese, phone first as the collection lists them.
        let mut fsts: Vec<&String> = names.iter().filter(|n| n.ends_with(".fst")).collect();
        fsts.sort_by_key(|n| (!n.starts_with("phone"), n.to_string()));
        let fars = join(names.iter().filter(|n| n.ends_with(".far")).collect());
        // A model with US and British lexicons reads with the one its voice speaks.
        let lexicons = {
            let language = v.language.to_lowercase();
            let british = language.contains("british") || language.contains("scottish");
            let has = |s: &str| names.iter().any(|n| n.starts_with("lexicon") && n.contains(s));
            // With both English lexicons, the other accent's is left out.
            let other = match (british, has("us-en") && has("gb-en")) {
                (_, false) => None,
                (true, true) => Some("us-en"),
                (false, true) => Some("gb-en"),
            };
            join(
                names
                    .iter()
                    .filter(|n| n.starts_with("lexicon") && n.ends_with(".txt"))
                    .filter(|n| other.is_none_or(|o| !n.contains(o)))
                    .collect(),
            )
        };
        let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4)) as i32;
        let mut model = OfflineTtsModelConfig { num_threads: threads, ..Default::default() };
        match kind(&dir).ok_or("The voice's files are incomplete.")? {
            Kind::Vits => {
                model.vits = OfflineTtsVitsModelConfig {
                    model: first(&|n| onnx(n)),
                    tokens: some("tokens.txt"),
                    lexicon: some("lexicon.txt"),
                    data_dir: some("espeak-ng-data"),
                    dict_dir: some("dict"),
                    ..Default::default()
                }
            }
            Kind::Kokoro => {
                model.kokoro = OfflineTtsKokoroModelConfig {
                    model: first(&|n| onnx(n)),
                    voices: some("voices.bin"),
                    tokens: some("tokens.txt"),
                    data_dir: some("espeak-ng-data"),
                    dict_dir: some("dict"),
                    lexicon: lexicons,
                    ..Default::default()
                }
            }
            Kind::Kitten => {
                model.kitten = OfflineTtsKittenModelConfig {
                    model: first(&|n| onnx(n)),
                    voices: some("voices.bin"),
                    tokens: some("tokens.txt"),
                    data_dir: some("espeak-ng-data"),
                    ..Default::default()
                }
            }
            Kind::Matcha => {
                model.matcha = OfflineTtsMatchaModelConfig {
                    acoustic_model: first(&|n| onnx(n) && n.starts_with("model")).or_else(|| first(&|n| onnx(n))),
                    vocoder: Some(vocoder_path(&v.model).to_string_lossy().into_owned()),
                    tokens: some("tokens.txt"),
                    lexicon: some("lexicon.txt"),
                    data_dir: some("espeak-ng-data"),
                    dict_dir: some("dict"),
                    ..Default::default()
                }
            }
        }
        Ok(OfflineTtsConfig { model, rule_fsts: join(fsts), rule_fars: fars, max_num_sentences: 1, ..Default::default() })
    }

    /// Speech for `text` in voice `v`: samples and their rate.
    pub fn render(v: &Voice, text: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
        if !engine_present() {
            return Err("The natural voice's files are missing from the app. Reinstall Voice to use it.".into());
        }
        let mut slot = ENGINE.lock().unwrap();
        if slot.as_ref().is_none_or(|(model, _)| *model != v.model) {
            *slot = None;
            let tts = OfflineTts::create(&config(v)?).ok_or("The voice's model couldn't be loaded.")?;
            *slot = Some((v.model.clone(), tts));
        }
        let tts = &slot.as_ref().expect("loaded").1;
        let speakers = tts.num_speakers();
        if speakers > 0 && v.speaker >= speakers {
            return Err(format!("This model has speakers 0 to {}; {} isn't one.", speakers - 1, v.speaker));
        }
        let generation = GenerationConfig { speed: speed.clamp(0.5, 2.0), sid: v.speaker, ..Default::default() };
        let audio = tts.generate_with_config(text, &generation, None::<fn(&[f32], f32) -> bool>).ok_or("The voice couldn't say that.")?;
        Ok((audio.samples().to_vec(), audio.sample_rate().max(1) as u32))
    }

    /// Lets go of the voice, before its files are deleted.
    pub fn unload() {
        *ENGINE.lock().unwrap() = None;
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn render(_v: &Voice, _text: &str, _speed: f32) -> Result<(Vec<f32>, u32), String> {
        Err("Natural voices aren't available on this device yet.".into())
    }

    pub fn unload() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_are_listed_recommended_then_by_language_gender_and_name() {
        let mut all = built_in();
        sort(&mut all);
        let names: Vec<&str> = all.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names[..2], ["Alba", "Joe"]);
        let english: Vec<&str> = all.iter().filter(|v| v.langs.iter().any(|l| l == "en")).map(|v| v.name.as_str()).collect();
        assert_eq!(english, ["Alba", "Joe", "Cori", "Jenny", "Alan", "Northern English", "Chá (茶)", "Yǔxuān (宇轩)", "Amy", "Lessac", "Henrik", "Norman", "Ryan"]);
        assert!(names.contains(&"Dave") && names.contains(&"Siwis"));
        assert_eq!(first_for("es").unwrap().name, "Dave");
        assert_eq!(first_for("en").unwrap().name, "Alba");
        let mut ids: Vec<_> = all.iter().map(|v| &v.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
    }

    #[test]
    fn every_offered_voice_has_a_pinned_model() {
        for v in built_in() {
            let m = MODELS.iter().find(|m| m.name == v.model).unwrap_or_else(|| panic!("{}", v.model));
            assert_eq!(m.sha256.len(), 64);
            assert!(v.size >= m.size);
        }
    }

    #[test]
    fn languages_are_read_from_their_names() {
        assert_eq!(langs_of("Chinese English"), ["en", "zh"]);
        assert_eq!(langs_of("Cymraeg / Welsh"), ["cy"]);
        assert!(langs_of("Klingon").is_empty());
    }
}
