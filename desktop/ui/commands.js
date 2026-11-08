"use strict";

/// Works out which command someone meant. Each command is described by the kinds of words that can say it,
/// rather than fixed sentences, so rephrasings and small mishearings still work ("start dictation", "jot this
/// down", "could you open my settings please"). A near miss comes back with a lower score, so the app can ask
/// "Did you mean…?" instead of refusing.

/// `groups`: one word from each group says the command. `alone`: any of these words says it by itself.
/// `phrases`: any of these anywhere says it. `start`: the whole request starts like this.
const COMMANDS = [
  { id: "cancel", say: "Cancel", start: /^(cancel|never ?mind|nevermind|stop listening|close|go away|that's all|that is all|nothing|no thanks)\b/ },
  { id: "stop-recording", say: "Stop recording", groups: [["stop", "end", "finish", "halt", "wrap"], ["record", "recording", "meeting", "recorder", "call"]] },
  { id: "record", say: "Record a meeting", groups: [["start", "begin", "record", "new", "kick"], ["record", "recording", "meeting", "call"]] },
  { id: "stop-note", say: "Stop the note", groups: [["stop", "end", "finish", "done"], ["note", "dictation", "dictating", "writing"]] },
  { id: "add-note", say: "Add to last note", groups: [["add", "append", "continue", "resume", "carry", "reopen"], ["last", "latest", "previous", "recent"], ["note", "notes"]] },
  { id: "read-note", say: "Read back my last note", groups: [["read", "play", "tell", "repeat"], ["note", "notes"]] },
  { id: "note", say: "Take a note", alone: ["dictate", "dictation", "dictating", "memo"], phrases: ["note this", "write this down", "write that down", "jot this down", "jot that down"],
    groups: [["take", "start", "new", "make", "write", "create", "begin", "open", "jot"], ["note", "dictation", "memo"]] },
  { id: "bookmark", say: "Bookmark that", alone: ["bookmark", "bookmarked"], groups: [["mark", "flag", "remember", "highlight"], ["that", "this", "it", "moment"]] },
  { id: "action", say: "Action item", groups: [["action", "todo", "task"], ["item", "items", "point"]] },
  { id: "help", say: "What can I say?", alone: ["help", "commands"], phrases: ["what can i say", "what can you do", "what do you do", "what are my options"],
    groups: [["what", "how"], ["say", "can", "do", "works"]] },
  { id: "settings", say: "Open settings", alone: ["settings", "preferences"] },
  { id: "home", say: "Show my notes", alone: ["home"], groups: [["show", "open", "go", "list", "see"], ["notes", "meetings", "everything"]] },
];

/// What is suggested when nothing comes close: meeting controls while recording, otherwise ways to start.
const SUGGESTED = ["record", "note", "help"];
const SUGGESTED_RECORDING = ["bookmark", "action", "stop-recording", "help"];

/// The wake word and the ways whisper writes it, which may lead a command.
const WAKE_WORDS = /\b(?:(?:hey|hi|hay|okay|ok)[\s,.!]+)?super[\s-]?t(?:ee|ea|he|i)m\w*[\s,.!]*/g;

function normalise(text) {
  return text
    .toLowerCase()
    .replace(/[’‘]/g, "'")
    .replace(WAKE_WORDS, " ")
    .replace(/\bbook ?marks?\b/g, "bookmark")
    .replace(/\bto ?dos?\b/g, "todo")
    .replace(/\b(\w+)'s\b/g, "$1")
    .replace(/[^\w\s']/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/// Whether a heard word is a known one, allowing endings ("recorded") and one letter wrong in longer words.
function wordMatches(word, known) {
  if (word === known) return true;
  if (known.length >= 5 && word.startsWith(known)) return true;
  if (word.length < 5 || known.length < 5 || Math.abs(word.length - known.length) > 1) return false;
  let i = 0;
  let j = 0;
  let edits = 0;
  while (i < word.length && j < known.length) {
    if (word[i] === known[j]) { i++; j++; continue; }
    if (++edits > 1) return false;
    if (word.length > known.length) i++;
    else if (word.length < known.length) j++;
    else { i++; j++; }
  }
  return edits + (word.length - i) + (known.length - j) <= 1;
}

/// How well `words` say `command`: 1 when they say it, 0.5 or so for half of it, 0 for nothing.
function scoreCommand(command, text, words) {
  if (command.start?.test(text)) return { score: 1, specific: 3 };
  if (command.phrases?.some((p) => text.includes(p))) return { score: 1, specific: 2 };
  let best = { score: 0, specific: 0 };
  if (command.alone && words.some((w) => command.alone.some((k) => wordMatches(w, k)))) best = { score: 1, specific: 1 };
  if (command.groups) {
    // Each word answers for one group, so "stop recording" isn't also "record … recording".
    const used = new Set();
    let matched = 0;
    for (const group of command.groups) {
      const at = words.findIndex((w, i) => !used.has(i) && group.some((k) => wordMatches(w, k)));
      if (at >= 0) {
        used.add(at);
        matched++;
      }
    }
    const score = matched / command.groups.length;
    if (score > best.score || (score === best.score && matched > best.specific)) best = { score, specific: matched };
  }
  return best;
}

/// Every command, best match first, each with `id`, `say` and `score`.
function interpret(raw) {
  const text = normalise(raw);
  const words = text.split(" ").filter(Boolean);
  return COMMANDS.map((c) => ({ id: c.id, say: c.say, ...scoreCommand(c, text, words) }))
    .sort((a, b) => b.score - a.score || b.specific - a.specific);
}

if (typeof module !== "undefined") module.exports = { interpret, normalise, wordMatches, COMMANDS, SUGGESTED, SUGGESTED_RECORDING };
