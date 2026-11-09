"use strict";

/// Works out which command someone meant. Each command is described by the kinds of words that can say it,
/// rather than fixed sentences, so rephrasings and small mishearings still work ("start dictation", "jot this
/// down", "could you open my settings please"). A near miss comes back with a lower score, so the app can ask
/// "Did you mean…?" instead of refusing.

/// `offer`: what it does, as in "To record a new note, say yes".
/// `groups`: one word from each group says the command. `alone`: any of these words says it by itself.
/// `phrases`: any of these anywhere says it. `start`: the whole request starts like this.
const COMMANDS = [
  { id: "cancel", offer: "stop listening", say: "Cancel", start: /^(cancel|never ?mind|nevermind|stop listening|close|go away|that's all|that is all|nothing|no thanks)\b/ },
  { id: "stop-recording", offer: "stop the recording", say: "Stop recording", groups: [["stop", "end", "finish", "halt", "wrap"], ["record", "recording", "meeting", "recorder", "call"]] },
  { id: "record", offer: "start recording a meeting", say: "Record a meeting", groups: [["start", "begin", "record", "new", "kick"], ["record", "recording", "meeting", "call"]] },
  { id: "stop-note", offer: "stop the note", say: "Stop the note", groups: [["stop", "end", "finish", "done"], ["note", "dictation", "dictating", "writing"]] },
  { id: "add-note", offer: "add more to your last note", say: "Add to last note", groups: [["add", "append", "continue", "resume", "carry", "reopen"], ["last", "latest", "previous", "recent"], ["note", "notes"]] },
  { id: "read-note", offer: "hear your last note read back", say: "Read back my last note", groups: [["read", "play", "tell", "repeat"], ["note", "notes"]] },
  { id: "note", offer: "record a new note", say: "Take a note", alone: ["dictate", "dictation", "dictating", "memo"], phrases: ["note this", "write this down", "write that down", "jot this down", "jot that down"],
    groups: [["take", "start", "new", "make", "write", "create", "begin", "open", "jot"], ["note", "dictation", "memo"]] },
  { id: "bookmark", offer: "bookmark this moment", say: "Bookmark that", alone: ["bookmark", "bookmarked"], groups: [["mark", "flag", "remember", "highlight"], ["that", "this", "it", "moment"]] },
  { id: "action", offer: "mark an action item", say: "Action item", groups: [["action", "todo", "task"], ["item", "items", "point"]] },
  { id: "help", offer: "hear what you can say", say: "What can I say?", alone: ["help", "commands"], phrases: ["what can i say", "what can you do", "what do you do", "what are my options"],
    groups: [["what", "how"], ["say", "can", "do", "works"]] },
  { id: "update", offer: "update the app to the newest version", say: "Update the app", alone: ["update", "upgrade"],
    phrases: ["new version", "newer version", "latest version", "check for updates"] },
  // Only offered once a newer version is found, never matched from words.
  { id: "install-update", offer: "install it now", say: "Install the update" },
  { id: "settings", offer: "open settings", say: "Open settings", alone: ["settings", "preferences"] },
  { id: "home", offer: "show your notes and meetings", say: "Show my notes", alone: ["home"], groups: [["show", "open", "go", "list", "see"], ["notes", "meetings", "everything"]] },
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

/// Subjects a half-heard request can be about, so the app can ask "Do you want to do something with a
/// note?" and then offer that subject's commands one at a time. `words` include ways whisper mishears them.
const TOPICS = [
  { id: "note", name: "a note", words: ["note", "notes", "noted", "node", "nodes", "dictate", "dictation", "memo", "jot", "write", "writing"],
    commands: ["note", "add-note", "read-note"] },
  { id: "meeting", name: "a meeting", words: ["record", "recording", "recorder", "meeting", "meetings", "call"],
    commands: ["record", "home"], recording: ["stop-recording", "bookmark", "action"] },
];

/// The one subject `raw` is about, with the commands that fit now, or null when it's about none or several.
function topicOf(raw, recording = false) {
  const words = normalise(raw).split(" ").filter(Boolean);
  const found = TOPICS.filter((t) => words.some((w) => t.words.some((k) => wordMatches(w, k))));
  if (found.length !== 1) return null;
  const t = found[0];
  return { id: t.id, name: t.name, commands: (recording && t.recording) || t.commands };
}

const YES = ["yes", "yeah", "yea", "yeh", "yep", "yup", "ya", "yah", "yas", "yess", "yet", "jess", "sure", "ok", "okay", "alright",
  "correct", "right", "please", "affirmative", "definitely", "absolutely", "certainly", "exactly"];
const YES_PHRASES = ["go ahead", "do it", "do that", "that one", "uh huh", "sounds good", "let's do it"];
const NO = ["no", "nope", "nah", "know", "negative", "wrong", "next", "neither", "other", "don't", "dont"];
const UNSURE = /\b(not sure|don't know|dont know|no idea|unsure|maybe)\b/;
const REPEAT = /^(repeat|say (that|it) again|again|pardon|sorry|what|come again|what was (that|the question))\b/;
const NUMBERS = { one: 1, won: 1, first: 1, 1: 1, two: 2, second: 2, 2: 2, three: 3, tree: 3, third: 3, 3: 3,
  four: 4, fourth: 4, 4: 4, five: 5, fifth: 5, 5: 5 };
/// Sound-alikes of numbers that are only taken as one when said by themselves.
const NUMBERS_ALONE = { to: 2, too: 2, free: 3, for: 4, fore: 4 };

/// A short answer to a question the app asked: "yes", "no", "cancel", "repeat", "unsure", a number from 1,
/// or null when it isn't one (a new request, or nothing understood). Generous, as answers are often misheard.
function answerOf(raw) {
  const text = normalise(raw);
  if (!text) return null;
  if (COMMANDS[0].start.test(text) || /^stop\b/.test(text)) return "cancel";
  if (REPEAT.test(text)) return "repeat";
  if (UNSURE.test(text)) return "unsure";
  const words = text.split(" ");
  if (words.length > 6) return null;
  if (words.length === 1 && words[0] in NUMBERS_ALONE) return NUMBERS_ALONE[words[0]];
  if (words.length <= 3 && !words.includes("not")) {
    const number = words.find((w) => w in NUMBERS);
    if (number) return NUMBERS[number];
  }
  const yes = words.some((w) => YES.includes(w)) || YES_PHRASES.some((p) => text.includes(p));
  const no = words.some((w) => NO.includes(w)) || /\bnot\b/.test(text);
  if (yes !== no) return yes ? "yes" : "no";
  return null;
}

if (typeof module !== "undefined") module.exports = { interpret, normalise, wordMatches, topicOf, answerOf, COMMANDS, TOPICS, SUGGESTED, SUGGESTED_RECORDING };
