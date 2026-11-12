//! A conversation about one note with the model that runs analyses: questions about what the note says,
//! and rewrites of it. A rewrite becomes a new version of the note (see notes.rs), never a silent change.
//! The conversation is kept beside the note as `<id>.chat.json`.

use crate::{library, notes};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use stv_core::analysis::{Model, CANCELLED};

/// The job name a chat answer waits under in the analysis queue.
pub const JOB: &str = "chat";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Message {
    /// "user" or "assistant".
    pub role: String,
    pub text: String,
    pub at: u64,
    /// The version of the note this answer made, when it rewrote it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// Why there's no answer, shown in its place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Chat {
    pub messages: Vec<Message>,
}

fn path(id: &str) -> Result<PathBuf, String> {
    notes::load(id)?;
    Ok(library::data_dir().join("notes").join(format!("{id}.chat.json")))
}

pub fn load(id: &str) -> Chat {
    path(id).ok().and_then(|p| library::read_json(&p).ok()).unwrap_or_default()
}

fn save(id: &str, chat: &Chat) -> Result<(), String> {
    let file = path(id)?;
    let text = serde_json::to_string_pretty(chat).map_err(|e| e.to_string())?;
    let partial = file.with_extension("json.tmp");
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &file).map_err(|e| e.to_string())
}

pub fn clear(id: &str) -> Result<(), String> {
    let file = path(id)?;
    if file.exists() {
        std::fs::remove_file(file).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Adds what the person said, to be answered by `answer`.
pub fn ask(id: &str, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Type a question or a request first.".into());
    }
    let mut chat = load(id);
    chat.messages.push(Message { role: "user".into(), text: text.into(), at: library::now(), ..Default::default() });
    save(id, &chat)
}

/// True while the last thing said is the person's, with no answer yet.
pub fn waiting(chat: &Chat) -> bool {
    chat.messages.last().is_some_and(|m| m.role == "user")
}

const ROLE: &str = "You are the assistant built into a notes app. You help the person with the one note shown to you: \
you answer questions about it, and you rewrite it when asked. Use only the note and this conversation; when the note \
doesn't say something, say so plainly. Write in the language the person writes to you in, unless they ask for another.\n\n\
Answer with JSON, in one of two forms.\n\
To answer a question, summarise, explain or suggest: {\"action\":\"answer\",\"reply\":\"your answer\"}\n\
When the person asks you to change the note itself (rewrite, shorten, expand, fix, tidy, restructure, change its tone, \
translate it, add or remove something): {\"action\":\"rewrite\",\"note\":\"the complete new text of the note\",\"reply\":\"one short sentence saying what you changed\"}\n\
In a rewrite, \"note\" holds the whole note from first line to last, ready to replace the old one, with no commentary \
and nothing left out that wasn't asked to go. The note is written in Markdown (# headings, - lists, - [ ] to-dos, \
**bold**, tables), and so is your rewrite. Pictures and files appear as ![words](attachment:name) and \
[words](attachment:name): keep each of those exactly as it is, where it belongs.";

/// Said instead of offering a rewrite when the whole note wouldn't fit in what the model has left to write.
const TOO_LONG_TO_REWRITE: &str = "\n\nThis note is too long for you to rewrite in one go, so only the first form is open to you. \
If the person asks you to change the note, say plainly that it is too long to rewrite whole here, and offer to help \
with one part of it at a time (they can paste that part into the chat), or suggest choosing a larger model in Settings.";

/// What the model is told it is, and which forms of answer it has.
fn system(can_rewrite: bool) -> String {
    if can_rewrite { ROLE.into() } else { format!("{ROLE}{TOO_LONG_TO_REWRITE}") }
}

/// Whether a rewrite of `body` fits in `room` tokens of output: the whole note again, with some to spare
/// for the reply, JSON escaping and a model that writes a little more than it was given.
fn rewrite_fits(body: &str, room: usize, count: impl Fn(&str) -> usize) -> bool {
    room >= count(body) * 6 / 5 + 120
}

/// A grammar for the forms of answer open to the model.
fn grammar(can_rewrite: bool) -> String {
    let root = if can_rewrite { "root ::= answer | rewrite" } else { "root ::= answer" };
    format!(
        r#"{root}
answer ::= "{{" ws "\"action\"" ws ":" ws "\"answer\"" ws "," ws "\"reply\"" ws ":" ws string ws "}}"
rewrite ::= "{{" ws "\"action\"" ws ":" ws "\"rewrite\"" ws "," ws "\"note\"" ws ":" ws string ws "," ws "\"reply\"" ws ":" ws string ws "}}"
ws ::= [ \t\n]{{0,8}}
string ::= "\"" ( [^"\\\x00-\x1f] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F]{{4}}) )* "\""
"#
    )
}

/// The same forms for a server. Strict structured output needs every field listed as required, so `note`
/// is null in an answer rather than left out: otherwise a server may copy the whole note into every answer.
fn schema(can_rewrite: bool) -> Value {
    let actions: &[&str] = if can_rewrite { &["answer", "rewrite"] } else { &["answer"] };
    json!({
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": actions},
            "reply": {"type": "string"},
            "note": {
                "type": ["string", "null"],
                "description": "null when action is \"answer\"; the complete new text of the note when action is \"rewrite\"",
            },
        },
        "required": ["action", "reply", "note"],
        "additionalProperties": false,
    })
}

/// What the model is shown: the note, as much of the conversation as fits in `history_budget` tokens
/// (the latest of it), and what the person just said.
fn prompt(note: &notes::Note, earlier: &[Message], said: &str, history_budget: usize, count: impl Fn(&str) -> usize) -> String {
    let mut out = String::new();
    let title = note.title.trim();
    if title.is_empty() {
        out += "The note:\n";
    } else {
        out += &format!("The note, titled \"{title}\":\n");
    }
    out += "<<<\n";
    out += if note.body.trim().is_empty() { "(The note is empty.)" } else { note.body.trim_end() };
    out += "\n>>>\n\n";
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0;
    for m in earlier.iter().rev().filter(|m| m.error.is_none() && !m.text.trim().is_empty()) {
        let who = if m.role == "user" { "Person" } else { "Assistant" };
        let line = match m.version {
            Some(_) => format!("{who}: (rewrote the note) {}", m.text.trim()),
            None => format!("{who}: {}", m.text.trim()),
        };
        used += count(&line) + 1;
        if used > history_budget {
            break;
        }
        lines.push(line);
    }
    if !lines.is_empty() {
        lines.reverse();
        out += "Earlier in this conversation:\n";
        out += &lines.join("\n");
        out += "\n\n";
    }
    out += &format!("The person now says: {said}");
    out
}

/// What came back: an answer, or a new text for the note with a line about it.
#[derive(Debug, PartialEq)]
enum Reply {
    Answer(String),
    Rewrite { note: String, reply: String },
}

fn parse(answer: &str) -> Result<Reply, String> {
    const CUT: &str = "The answer was cut short. The note may be too long for this model to rewrite in one go: try asking about a part of it, or choose a larger model in Settings.";
    let start = answer.find('{').ok_or(CUT)?;
    let end = answer.rfind('}').ok_or(CUT)?;
    let v: Value = serde_json::from_str(&answer[start..=end]).map_err(|_| CUT.to_string())?;
    let text = |key: &str| v[key].as_str().unwrap_or("").trim().to_string();
    let (note, reply) = (text("note"), text("reply"));
    if v["action"] == "rewrite" && !note.is_empty() {
        Ok(Reply::Rewrite { note, reply: if reply.is_empty() { "I rewrote the note.".into() } else { reply } })
    } else if !reply.is_empty() {
        Ok(Reply::Answer(reply))
    } else {
        Err("The model gave no answer. Try asking again.".into())
    }
}

/// Answers the last thing the person said, rewriting the note when that's what was asked.
pub fn answer(id: &str, model: &mut dyn Model) -> Result<(), String> {
    let mut chat = load(id);
    let Some((said, earlier)) = chat.messages.split_last().filter(|(m, _)| m.role == "user") else {
        return Ok(());
    };
    let outcome = (|| {
        let note = notes::load(id)?;
        let context = model.context();
        let user = prompt(&note, earlier, &said.text, context / 8, |t| model.count(t));
        // What the chat format itself takes, and a little to spare.
        let used = model.count(&system(false)) + model.count(&user) + 160;
        if used + 200 > context {
            return Err("This note is too long for the model to read in one go. Choose a model with more room in Settings, or try with a shorter note.".to_string());
        }
        // A rewrite is the whole note again: offered only when that fits, so a long note gets an answer
        // saying so instead of a rewrite cut off after the model has spent minutes on it.
        let can_rewrite = rewrite_fits(&note.body, context - used, |t| model.count(t));
        match parse(&model.complete(&system(can_rewrite), &user, &grammar(can_rewrite), &schema(can_rewrite), context - used)?)? {
            Reply::Answer(text) => Ok((text, None)),
            Reply::Rewrite { note: body, reply } => {
                if body.trim() == note.body.trim() {
                    return Ok((format!("{reply} (The text came out the same, so the note wasn't changed.)"), None));
                }
                let label: String = said.text.chars().take(200).collect();
                let version = notes::rewrite(id, body, &label)?;
                Ok((reply, Some(version)))
            }
        }
    })();
    let now = library::now();
    let reply = match outcome {
        Ok((text, version)) => Message { role: "assistant".into(), text, at: now, version, error: None },
        Err(e) => Message { role: "assistant".into(), text: String::new(), at: now, version: None, error: Some(shown(&e).into()) },
    };
    chat.messages.push(reply);
    save(id, &chat)
}

/// Why there's no answer, as the chat shows it. Stopped by the person: what they asked stays, with
/// "Stopped." under it, whether the model was loading or writing.
fn shown(why: &str) -> &str {
    if why == CANCELLED { "Stopped." } else { why }
}

/// An answer that never came, as when the app was closed mid-way, is marked so the chat isn't left waiting.
pub fn settle(id: &str, why: Option<&str>) -> Result<(), String> {
    let mut chat = load(id);
    if waiting(&chat) {
        let why = why.map_or("Stopped before it was answered.", shown);
        chat.messages.push(Message { role: "assistant".into(), at: library::now(), error: Some(why.into()), ..Default::default() });
        save(id, &chat)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_and_rewrites_are_told_apart() {
        assert_eq!(parse(r#"{"action":"answer","reply":"Three people."}"#), Ok(Reply::Answer("Three people.".into())));
        assert_eq!(
            parse(r#"{"action":"rewrite","note":"Line one\nLine two","reply":"Shortened it."}"#),
            Ok(Reply::Rewrite { note: "Line one\nLine two".into(), reply: "Shortened it.".into() })
        );
        // A server that fills every field: an empty note is an answer.
        assert_eq!(parse(r#"{"action":"answer","note":"","reply":"Yes."}"#), Ok(Reply::Answer("Yes.".into())));
        assert_eq!(parse(r#"{"action":"answer","reply":"Yes.","note":null}"#), Ok(Reply::Answer("Yes.".into())));
        assert!(parse(r#"{"action":"rewrite","note":"Half a no"#).is_err());
        assert!(parse(r#"{"action":"answer","reply":""}"#).is_err());
        // Stopped while loading or writing reads the same.
        assert_eq!(shown(CANCELLED), "Stopped.");
        assert_eq!(shown("The model file is missing."), "The model file is missing.");
    }

    #[test]
    fn a_rewrite_is_offered_only_when_the_whole_note_fits() {
        let words = |t: &str| t.split_whitespace().count();
        let body = "word ".repeat(1000);
        assert!(rewrite_fits(&body, 1400, words));
        assert!(!rewrite_fits(&body, 1000, words));
        // Without room, the model can only answer, and is told why.
        assert!(grammar(true).starts_with("root ::= answer | rewrite\n"));
        assert!(grammar(false).starts_with("root ::= answer\n"));
        assert_eq!(schema(false)["properties"]["action"]["enum"], json!(["answer"]));
        assert!(system(false).contains("too long for you to rewrite") && !system(true).contains("too long for you to rewrite"));
    }

    #[test]
    fn the_prompt_keeps_the_latest_of_a_long_conversation() {
        let note = notes::Note { title: "Plan".into(), body: "Ship on Friday.".into(), ..Default::default() };
        let said = |role: &str, text: &str| Message { role: role.into(), text: text.into(), ..Default::default() };
        let earlier = [said("user", "first question"), said("assistant", "first answer"), said("user", "second question"), said("assistant", "second answer")];
        let words = |t: &str| t.split_whitespace().count();
        let all = prompt(&note, &earlier, "When do we ship?", 100, words);
        assert!(all.contains("titled \"Plan\"") && all.contains("Ship on Friday.") && all.ends_with("The person now says: When do we ship?"));
        assert!(all.contains("Person: first question\nAssistant: first answer"));
        let some = prompt(&note, &earlier, "When do we ship?", 9, words);
        assert!(!some.contains("first question") && some.contains("Assistant: second answer"));
    }
}
