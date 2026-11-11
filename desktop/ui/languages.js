"use strict";

/// Languages: the picker beside a note's or meeting's status (one language, or two), Settings › Languages
/// with the speech models, and the translation panel on a meeting or note.

icon.globe = '<circle cx="12" cy="12" r="9"></circle><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"></path>';

/// The person's languages and the catalogue, fetched once and again after a change:
/// { all: [[code, name]], catalogue: [{ code, name, native, rtl, speech, analysis }], chosen, primary,
///   defaults, translate_into, auto_translate, speech_model }.
let languageInfo = null;
async function languageData(force) {
  if (force || !languageInfo) {
    languageInfo = await invoke("get_languages").catch(() => languageInfo ?? { all: [["en", "English"]], catalogue: [], chosen: ["en"], primary: "en", defaults: [], translate_into: [] });
  }
  return languageInfo;
}
languageData();

const languageEntry = (code) => languageInfo?.catalogue?.find((l) => l.code === code);
const languageName = (code) => languageEntry(code)?.name ?? languageInfo?.all?.find(([c]) => c === code)?.[1] ?? code;

/// The tag a voice or spell-checker knows a language by.
const languageTag = (code) => ({ zh: "zh-CN", "zh-Hant": "zh-TW", yue: "zh-HK", fil: "fil-PH" })[code] ?? code;
const isRightToLeft = (code) => !!languageEntry(code)?.rtl || ["ar", "he", "fa", "ur"].includes(code);

/// Marks text as being in `code`, for spell-checking and so right-to-left languages run that way.
function setLanguage(nodes, code) {
  for (const node of nodes) {
    if (!node) continue;
    node.lang = languageTag(code);
    node.dir = isRightToLeft(code) ? "rtl" : "ltr";
  }
}

/// The languages an item is in, main one first: its own, or the primary one.
function itemLanguages(item) {
  const own = item?.languages?.length ? item.languages : item?.language ? [item.language] : [];
  return own.length ? own : [languageInfo?.primary ?? "en"];
}

const QUALITY = { good: "Transcribes well", fair: "Transcribes fairly well", weak: "Transcription is rough", none: "Can't be transcribed yet" };

/// Options for a language <select>: the person's languages first, then every other one Voice knows.
function languageOptions(selected, { none } = {}) {
  const info = languageInfo ?? { chosen: ["en"], catalogue: [] };
  const mine = info.chosen;
  const others = (info.catalogue ?? []).filter((l) => !mine.includes(l.code));
  const option = (code, label) => el("option", { value: code, selected: code === selected }, label);
  return [
    none ? option("", none) : null,
    el("optgroup", { label: "Your languages" }, mine.map((c) => option(c, languageName(c)))),
    others.length ? el("optgroup", { label: "More languages" }, others.map((l) => option(l.code, l.native && l.native !== l.name ? `${l.name} · ${l.native}` : l.name))) : null,
  ];
}

/// "EN", "ZH": a language's short name, for the chip beside the status.
const languageShort = (code) => code.split("-")[0].toUpperCase();

/// The chip beside a note's or meeting's status, showing its language ("EN"), or two ("EN · ES") for one
/// where people switch between them. It opens a dialog to change them. `onChange(list)` saves and says
/// whether it worked.
function languagePicker(current, onChange) {
  let saved = [...current];
  const chip = el("button", { class: "language-chip", type: "button" });
  const draw = () => {
    const names = saved.map(languageName).join(" and ");
    chip.title = `In ${names}. Change the language`;
    chip.setAttribute("aria-label", `Language: ${names}. Change it`);
    chip.replaceChildren(svg("globe", 14), el("span", {}, saved.map(languageShort).join(" · ")));
  };
  chip.addEventListener("click", () => {
    const main = el("select", { "aria-label": "Language" }, languageOptions(saved[0]));
    const second = el("select", { "aria-label": "Second language" }, languageOptions(saved[1] ?? "", { none: "None" }));
    const block = () => { for (const opt of second.querySelectorAll("option")) opt.disabled = !!opt.value && opt.value === main.value; };
    main.addEventListener("change", () => { if (second.value === main.value) second.value = ""; block(); });
    block();
    const save = el("button", { class: "primary pill", type: "button" }, "Save");
    const dialog = el("dialog", { class: "sheet", "aria-label": "Language" },
      el("h2", {}, "Language"),
      el("label", { class: "field" }, "Spoken and written in", main),
      el("label", { class: "field" }, el("span", {}, "Second language", el("small", {}, "For when people switch between two.")), second),
      el("div", { class: "actions" }, el("button", { class: "pill", type: "button", onclick: () => dialog.close() }, "Cancel"), save));
    save.addEventListener("click", async () => {
      const list = [main.value, second.value].filter((c, i, a) => c && a.indexOf(c) === i);
      save.disabled = true;
      if (await onChange(list)) saved = list;
      draw();
      dialog.close();
    });
    dialog.addEventListener("close", () => dialog.remove());
    document.body.append(dialog);
    dialog.showModal();
  });
  draw();
  return chip;
}

/// Saves an item's languages and follows up: a new speech model to download, and for a meeting already
/// transcribed, transcribing it again. Messages go in `notice`. Returns the result, or null on failure.
async function changeItemLanguages(kind, id, list, notice) {
  let result;
  try {
    result = await invoke("set_item_languages", { kind, id, languages: list });
  } catch (e) {
    showError(notice, e);
    return null;
  }
  if (result.added.length) await languageData(true);
  notice.replaceChildren();
  notice.hidden = true;
  const names = result.effective.map(languageName).join(" and ");
  const lines = [];
  if (result.added.length) lines.push(el("p", {}, `${result.added.map(languageName).join(" and ")} ${result.added.length > 1 ? "are" : "is"} now among your languages.`));
  const weak = result.effective.map(languageEntry).filter((l) => l && (l.speech === "weak" || l.speech === "none"));
  if (weak.length) lines.push(el("p", { class: "hint" }, `${weak.map((l) => l.name).join(" and ")}: transcripts will be rough, so check them. Analyses are made from an English translation.`));
  const offerAgain = () => {
    if (kind !== "meeting" || !result.changed || !result.can_retranscribe) return null;
    return el("div", { class: "actions" }, el("button", { class: "primary", onclick: () => processAgain(id) }, `Transcribe again in ${names}`),
      el("button", { class: "link", onclick: () => { notice.hidden = true; } }, "Not now"));
  };
  if (!result.model.ready) {
    const size = sizeText(result.model.size);
    const status = el("p", { class: "hint" });
    const get = el("button", { class: "primary" }, `Download it (${size})`);
    const box = el("div", {}, el("p", {}, `${names} needs the ${result.model.name} speech model, ${size}. It's downloaded once and stays on this device.`), status,
      el("div", { class: "actions" }, get, el("button", { class: "link", onclick: () => { notice.hidden = true; } }, "Later")));
    get.addEventListener("click", async () => {
      get.disabled = true;
      const off = await listen("speech-model-download", (e) => { if (e.payload.id === result.model.id) status.textContent = `Downloading… ${Math.round(e.payload.fraction * 100)}%`; });
      try {
        await invoke("download_speech_model", { id: result.model.id });
        result.model.ready = true;
        box.replaceChildren(el("p", {}, `The ${result.model.name} speech model is ready.`), offerAgain());
      } catch (e) {
        get.disabled = false;
        status.textContent = String(e);
      } finally {
        off();
      }
    });
    lines.push(box);
  } else if (kind === "meeting" && result.changed && result.can_retranscribe) {
    lines.push(el("p", {}, `The transcript was made before this was set to ${names}.`), offerAgain());
  }
  if (result.reason && result.changed) lines.push(el("p", { class: "hint" }, result.reason));
  if (lines.length) {
    notice.replaceChildren(...lines.filter(Boolean));
    notice.hidden = false;
  }
  return result;
}

// ---------- Settings › Languages

/// Settings › Languages: the ones the person uses and the primary one, what new items start in, what to
/// translate into, and the speech models.
async function showLanguages(message) {
  leave();
  showPage();
  view = { kind: "languages" };
  refreshItems();
  const [data, models] = await Promise.all([languageData(true), invoke("speech_models").catch(() => [])]);
  const page = el("div", { class: "page" }, el("button", { class: "link back-link", onclick: () => showSettings() }, "‹ Settings"), el("h1", {}, "Languages"));
  page.append(el("p", { class: "hint" }, "The languages you use. A note or meeting is in your primary language unless you choose others for it, beside its status: one language, or two for when people switch between them."));
  if (message) page.append(el("div", { class: "notice" }, message));
  const save = async (chosen, primary) => {
    try { await invoke("save_languages", { languages: chosen, primary }); await languageData(true); showLanguages(); } catch (e) { showError(page, e); }
  };
  const settings = async (change) => {
    try { languageInfo = await invoke("save_language_settings", change); showLanguages(); } catch (e) { showError(page, e); }
  };

  const group = el("div", { class: "group" });
  for (const code of data.chosen) {
    const l = languageEntry(code);
    const primary = data.primary === code;
    const row = el("div", { class: "row language-row" },
      el("span", { class: "label" }, languageName(code), el("small", {}, [l ? QUALITY[l.speech] : "", l?.analysis === "via_english" ? "Analyses via English" : "Analyses in this language"].filter(Boolean).join(" · "))));
    if (primary) row.append(el("span", { class: "badge ok" }, "Primary"));
    else row.append(el("button", { class: "pill", onclick: () => save(data.chosen, code) }, "Make primary"),
      el("button", { class: "icon-only", title: `Remove ${languageName(code)}`, "aria-label": `Remove ${languageName(code)}`, onclick: () => save(data.chosen.filter((c) => c !== code), data.primary) }, svg("close", 14)));
    group.append(row);
  }
  const add = el("select", { "aria-label": "Add a language" }, el("option", { value: "" }, "Add a language…"),
    (data.catalogue ?? []).filter((l) => !data.chosen.includes(l.code)).map((l) => el("option", { value: l.code }, l.native && l.native !== l.name ? `${l.name} · ${l.native}` : l.name)));
  add.addEventListener("change", () => add.value && save([...data.chosen, add.value], data.primary));
  group.append(el("div", { class: "row add-language" }, add));
  page.append(group);

  // What new notes and meetings start in.
  const defaults = data.defaults.length ? data.defaults : [data.primary];
  page.append(el("h2", {}, "New notes and meetings"), el("div", { class: "group" },
    el("div", { class: "row" }, el("span", { class: "label" }, "Start in", el("small", {}, "Choose two for people who switch between languages, such as Chinese and English.")),
      languagePicker(defaults, async (list) => { await settings({ defaults: list }); return true; }))));

  // Translation.
  const others = data.chosen;
  const into = el("div", { class: "group" });
  if (others.length < 2) {
    into.append(el("div", { class: "row" }, el("span", { class: "label hint" }, "Add another language to translate into it.")));
  } else {
    for (const code of others) {
      const box = el("input", { type: "checkbox", checked: data.translate_into.includes(code), "aria-label": `Translate into ${languageName(code)}` });
      box.addEventListener("change", () => settings({ translateInto: box.checked ? [...data.translate_into, code] : data.translate_into.filter((c) => c !== code) }));
      into.append(el("label", { class: "row" }, box, el("span", { class: "label" }, languageName(code))));
    }
    const auto = el("input", { type: "checkbox", checked: data.auto_translate, "aria-label": "Translate meetings by themselves" });
    auto.addEventListener("change", () => settings({ autoTranslate: auto.checked }));
    into.append(el("label", { class: "row" }, auto, el("span", { class: "label" }, "Translate each meeting once it's processed",
      el("small", {}, "Into the languages ticked above, one meeting at a time. Translating uses the analysis language model and takes a few minutes."))));
  }
  page.append(el("h2", {}, "Translate into"), into);

  // Speech models live with the rest of transcription.
  const usedFor = models.filter((m) => m.ready && m.used_for.length).map((m) => m.name);
  page.append(el("h2", {}, "Speech models"), el("div", { class: "group" },
    linkRow("Speech models", usedFor.join(" and ") || "Choose and download them", () => showTranscription(), { drop: true })),
    el("p", { class: "hint" }, "Which model transcribes each of your languages, and downloading others, are in Settings › Transcription."));
  main.replaceChildren(page);
}

/// One line for the Settings row.
async function languagesSummary() {
  const data = await languageData(true).catch(() => null);
  if (!data) return "English";
  const others = data.chosen.filter((c) => c !== data.primary).map(languageName);
  return [languageName(data.primary), ...others].join(", ");
}

// ---------- Translations

/// The translations on the page now (see translationStack).
let translationStackNow = null;

/// The languages an item can be translated into: the person's, except the one it's all in.
const translationTargets = (languages) => (languageInfo?.chosen ?? []).filter((c) => !(languages.length === 1 && languages[0] === c));

/// A meeting's or note's translations, one panel each, shown under its analyses. There are none until one
/// is made with Translate, beside Analysis templates. `languages` are the item's, main one first;
/// `ctx.go(line)` jumps to a line in the original.
function translationStack(kind, id, languages, ctx = {}) {
  const holder = el("div", { class: "translations" });
  const stack = { kind, id, holder, languages, ctx, panels: new Map() };
  translationStackNow = stack;
  refreshTranslationStack(stack);
  return holder;
}

/// Adds a panel for each language translated, or being translated, that hasn't one.
async function refreshTranslationStack(stack) {
  const { kind, id } = stack;
  const [data, queue] = await Promise.all([
    invoke("get_translation", { kind, id, language: stack.languages[0] }).catch(() => null),
    invoke("get_analyses", { kind, id }).then((d) => d.queue).catch(() => ({ current: null, waiting: [] }))]);
  if (translationStackNow !== stack) return;
  const jobs = [queue.current, ...queue.waiting].filter((j) => j && j.kind === kind && j.id === id && j.template.startsWith("translate:"))
    .map((j) => j.template.slice("translate:".length));
  const own = stack.languages.length === 1 ? stack.languages[0] : null;
  for (const language of new Set([...(data?.available ?? []), ...jobs])) {
    if (language !== own) addTranslation(stack, language);
  }
  refreshAnalyses(); // So Translate leaves out the languages already here.
}

/// The panel for one language, made if it isn't there yet.
function addTranslation(stack, language, { open = false } = {}) {
  const had = stack.panels.get(language);
  if (had) {
    if (open && !had.open) { had.open = true; drawTranslation(had); }
    return had;
  }
  const holder = el("section", { class: "translation", "aria-label": `${languageName(language)} translation` });
  const page = { stack, kind: stack.kind, id: stack.id, holder, languages: stack.languages, ctx: stack.ctx, open, data: null, reading: null, language };
  // Made once, so it keeps reading and saying Stop while the panel is redrawn around it.
  page.follow = (piece, voice) => followReading(page, piece, voice);
  page.listen = readAloudButton(null, `Listen to the ${languageName(language)} translation`, { label: "Listen", lang: () => page.language, pieces: () => translationSpeech(page), onPiece: page.follow });
  page.listen.classList.add("translation-listen");
  page.status = el("div", { class: "translation-reading", role: "status", "aria-live": "polite", hidden: true });
  stack.panels.set(language, page);
  stack.holder.append(holder);
  drawTranslation(page);
  return page;
}

/// Translates the item on the page into `language`, and shows its panel, open.
async function startTranslation(language) {
  const stack = translationStackNow;
  if (!stack?.holder.isConnected) return;
  const page = addTranslation(stack, language, { open: true });
  page.error = null;
  try { await invoke("translate_item", { kind: stack.kind, id: stack.id, language }); } catch (e) { page.error = String(e); }
  drawTranslation(page);
  refreshAnalyses();
}

/// "Translate", beside Analysis templates: the person's other languages, then any language.
function translateMenu(kind, id) {
  const stack = translationStackNow;
  if (!stack || stack.kind !== kind || stack.id !== id) return null;
  const label = el("span", { class: "with-icon" }, svg("globe", 16), "Translate");
  const targets = translationTargets(stack.languages).filter((c) => !stack.panels.has(c));
  const choices = targets.map((c) => [languageName(c), null, () => startTranslation(c)]);
  choices.push(["Another language…", null, () => pickTranslationLanguage(stack)]);
  const menu = menuButton(label, choices, "link add-analysis");
  menu.classList.add("translate-menu");
  return menu;
}

/// A dialog to choose any language to translate into.
function pickTranslationLanguage(stack) {
  const own = stack.languages.length === 1 ? stack.languages[0] : null;
  const pick = el("select", { "aria-label": "Translate into" }, el("option", { value: "", selected: true, disabled: true }, "Choose a language"),
    ...languageOptions("").filter(Boolean));
  for (const opt of pick.querySelectorAll("option")) if (opt.value && (opt.value === own || stack.panels.has(opt.value))) opt.disabled = true;
  const go = el("button", { class: "primary pill", type: "button", disabled: true }, "Translate");
  pick.addEventListener("change", () => { go.disabled = !pick.value; });
  const dialog = el("dialog", { class: "sheet", "aria-label": "Translate" },
    el("h2", {}, "Translate"),
    el("p", { class: "hint" }, `Make a version of this ${stack.kind} in another language, to read, correct, export or listen to. Your analysis model translates it line by line.`),
    el("label", { class: "field" }, "Into", pick),
    el("div", { class: "actions" }, el("button", { class: "pill", type: "button", onclick: () => dialog.close() }, "Cancel"), go));
  go.addEventListener("click", () => { const language = pick.value; dialog.close(); startTranslation(language); });
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
}

/// The translation as pieces to read aloud, from line `from` on, each knowing its line. In a meeting, who
/// is speaking is said when it changes, as the transcript does.
function translationSpeech(page, from = 0) {
  const lines = page.data?.lines ?? [];
  const names = page.kind === "meeting" && (prefs.read_speakers ?? true);
  const pieces = [];
  let previous = null;
  lines.slice(from).forEach((line) => {
    if (!line.text) return;
    const who = names && line.speaker && line.speaker !== previous ? `${line.speaker}: ` : "";
    previous = line.speaker;
    for (const piece of speechPieces(who + line.text)) pieces.push({ ...piece, line: line.id });
  });
  return pieces;
}

/// Reads aloud from one line of the translation to the end.
function listenFrom(page, index) {
  stopReading();
  readAloud(page.listen, null, page.language, { pieces: translationSpeech(page, index), onPiece: page.follow });
}

/// Follows reading aloud: marks the line being read and says which, and in what voice.
function followReading(page, piece, voice) {
  page.reading = piece ? { line: piece.line, voice } : null;
  page.holder.querySelectorAll(".translation-line.speaking").forEach((n) => n.classList.remove("speaking"));
  drawReadingStatus(page);
  if (!piece) return;
  if (!page.open) {
    // The lines are shown, so the one being read can be followed; drawing marks it.
    page.open = true;
    drawTranslation(page);
    return;
  }
  markSpeaking(page);
}

function markSpeaking(page) {
  if (!page.reading) return;
  const node = page.holder.querySelector(`.translation-line[data-line="${CSS.escape(page.reading.line)}"]`);
  if (!node) return;
  node.classList.add("speaking");
  node.scrollIntoView({ block: "nearest", behavior: "smooth" });
}

/// "Reading line 4 of 30 in Spanish · Alba", and a warning when nothing on this device speaks the language.
function drawReadingStatus(page) {
  const { status, reading, data } = page;
  if (!reading || !data) { status.hidden = true; status.replaceChildren(); return; }
  const spoken = data.lines.filter((l) => l.text);
  const n = spoken.findIndex((l) => l.id === reading.line) + 1;
  const name = languageName(page.language);
  const voice = reading.voice ? reading.voice.name : "this device's voice";
  const parts = [el("span", { class: "translation-reading-dot", "aria-hidden": "true" }),
    el("span", {}, `Reading line ${n} of ${spoken.length} in ${name} · ${voice}`)];
  // The system's voices: with none for the language, it would be read in another language's accent.
  const base = languageTag(page.language).toLowerCase().split("-")[0];
  const systemVoices = window.speechSynthesis?.getVoices?.() ?? [];
  if (!reading.voice && platform !== "android" && systemVoices.length && !systemVoices.some((v) => v.lang.toLowerCase().startsWith(base))) {
    parts.push(el("small", { class: "translation-reading-warning" },
      `This device has no ${name} voice, so it may sound wrong. Add one in ${platform === "windows" ? "Windows Settings › Time & language › Speech" : "the system's speech settings"}.`));
  }
  status.replaceChildren(...parts);
  status.hidden = false;
}

/// When no natural voice that speaks the translation's language is downloaded but one is offered, a
/// one-click download of it, so Listen sounds right.
async function voiceOffer(page) {
  if (!isDesktop()) return null;
  const status = await invoke("tts_status").catch(() => null);
  if (!status?.supported) return null;
  const speaks = status.voices.filter((v) => v.langs?.includes(page.language));
  if (!speaks.length || speaks.some((v) => v.ready)) return null;
  const first = await invoke("offered_voice", { language: page.language }).catch(() => null);
  const voice = speaks.find((v) => v.id === first?.id) ?? speaks[0];
  const size = `${Math.round(voice.size / 1e6)} MB`;
  const name = languageName(page.language);
  const box = el("div", { class: "translation-voice" });
  const get = el("button", { class: "pill", type: "button", disabled: !!status.downloading }, `Get ${voice.name} (${size})`);
  get.addEventListener("click", async () => {
    box.replaceChildren(el("span", { class: "spinner" }), " ", `Downloading ${voice.name}… `, el("span", { "data-tts-progress": voice.id }, ""));
    try {
      await invoke("download_tts", { id: voice.id });
      await refreshPrefs();
    } catch (e) {
      page.error = String(e);
    }
    drawTranslation(page);
  });
  box.append(svg("speak", 16), el("span", { class: "grow" }, `Listen sounds best with a natural ${name} voice. ${voice.name} reads ${name} on this device, offline.`), get);
  return box;
}

/// Called by the job events: redraws the panel when its translation moves on or finishes.
function refreshTranslation(job, finished) {
  const stack = translationStackNow;
  if (!stack || !stack.holder.isConnected || job.kind !== stack.kind || job.id !== stack.id || !job.template.startsWith("translate:")) return;
  const page = addTranslation(stack, job.template.slice("translate:".length));
  if (finished) {
    page.progress = null;
    page.error = job.error ?? null;
    page.open = true;
  } else {
    page.progress = job.parts > 1 ? `Translating part ${job.part} of ${job.parts}…` : "Translating…";
  }
  drawTranslation(page);
}

async function drawTranslation(page) {
  const { kind, id, holder } = page;
  let data = null;
  try {
    data = await invoke("get_translation", { kind, id, language: page.language });
  } catch (e) {
    holder.replaceChildren(el("div", { class: "error" }, String(e)));
    return;
  }
  if (page.stack.panels.get(page.language) !== page) return;
  page.data = data;
  const queue = await invoke("get_analyses", { kind, id }).then((d) => d.queue).catch(() => ({ current: null, waiting: [] }));
  const job = `translate:${page.language}`;
  const running = queue.current?.kind === kind && queue.current?.id === id && queue.current?.template === job;
  const waiting = queue.waiting.some((j) => j.kind === kind && j.id === id && j.template === job);
  const busy = running || waiting;
  const total = data.lines.length;
  const translated = total - data.missing;
  const name = languageName(page.language);

  const translate = async () => {
    page.error = null;
    try { await invoke("translate_item", { kind, id, language: page.language }); page.open = true; } catch (e) { page.error = String(e); }
    drawTranslation(page);
  };
  // One thing to do at a time: translate, translate what's new, or wait for it.
  let state = null;
  let action = null;
  if (busy) {
    state = el("span", { class: "badge busy" }, el("span", { class: "spinner" }), running ? page.progress ?? "Loading the model…" : "Waiting its turn");
    action = el("button", { class: "link", onclick: () => invoke("cancel_analysis", { kind, id, template: job }) }, "Cancel");
  } else if (!translated) {
    state = el("span", { class: "hint" }, "Not translated yet");
    action = el("button", { class: "primary pill", onclick: translate }, "Translate");
  } else if (data.missing) {
    state = el("span", { class: "hint" }, `${translated} of ${total} lines`);
    action = el("button", { class: "pill", title: "Lines added or changed since it was translated", onclick: translate }, `Translate ${data.missing} new line${data.missing > 1 ? "s" : ""}`);
  } else {
    state = el("span", { class: "badge ok" }, svg("check", 12), "Up to date");
  }
  const exporter = translated ? exportMenu([["txt", "Translation as text", ".txt"], ["md", "Translation as Markdown", ".md"]],
    (format) => invoke("export_translation", { kind, id, language: page.language, format })) : null;
  const toggle = el("button", { class: "icon-only chevron", "aria-expanded": String(page.open), title: page.open ? "Hide the translation" : "Show the translation",
    onclick: () => { page.open = !page.open; drawTranslation(page); } }, svg(page.open ? "down" : "chevron", 16));
  const head = el("div", { class: "analysis-head translation-head" }, toggle, svg("globe", 16), el("strong", {}, name),
    el("span", { class: "translation-into" }, "translation"), state, el("span", { class: "grow" }),
    action, translated ? page.listen : null, exporter);
  page.listen.disabled = !translated;
  const parts = [head];
  if (exporter) parts.push(exporter.status);
  parts.push(page.status);
  if (page.error) parts.push(el("p", { class: "danger" }, page.error));
  if (data.advice) parts.push(el("p", { class: "hint" }, data.advice, " ", el("button", { class: "link inline", onclick: () => { analysisOrigin = { kind, id }; showAnalysisSettings(); } }, "Analysis settings")));

  const offer = translated ? await voiceOffer(page) : null;
  if (offer) parts.push(offer);
  if (page.open && translated) {
    const rows = el("div", { class: "translation-lines" });
    data.lines.forEach((line, index) => {
      const text = line.text ? editable(line.text, async (value) => {
        try { await invoke("edit_translation", { kind, id, language: page.language, line: line.id, text: value }); drawTranslation(page); } catch (e) { showError(holder, e); }
      }, "div") : el("div", { class: "hint" }, "Not translated yet");
      setLanguage([text], page.language);
      const original = line.same ? null : el("div", { class: "translation-original" }, line.original);
      setLanguage([original], page.languages[0]);
      const time = line.at_s != null ? el("button", { class: "link mono", title: "Go to this line in the transcript", onclick: () => page.ctx.go?.(line) }, clock(line.at_s)) : null;
      const flags = [];
      if (line.edited) flags.push(el("span", { class: "badge", title: "Corrected by you; translating again keeps it. Clear the text to go back to the machine translation." }, "Corrected"));
      if (line.original_changed) flags.push(el("span", { class: "badge stale", title: "The original line changed after you corrected this" }, "Original changed"));
      if (line.warning) flags.push(el("span", { class: "badge stale", title: line.warning }, "Check"));
      const who = time || line.speaker || flags.length ? el("div", { class: "translation-who" }, time, line.speaker ? el("strong", {}, line.speaker) : null, ...flags) : null;
      const here = line.text ? el("button", { class: "icon-only listen-here", type: "button", title: "Listen from here", "aria-label": "Listen from here",
        onmousedown: (e) => e.preventDefault(), onclick: () => listenFrom(page, index) }, svg("play", 14)) : null;
      rows.append(el("div", { class: `translation-line${line.same ? " same" : ""}`, "data-line": line.id },
        el("div", { class: "translation-content" }, who, text, original), here));
    });
    const remove = el("button", { class: "link danger", type: "button" }, "Delete this translation");
    remove.addEventListener("click", async () => {
      if (!remove.dataset.sure) {
        remove.dataset.sure = "1";
        remove.textContent = `Delete the ${name} translation and your corrections? Click again to delete.`;
        setTimeout(() => { if (remove.isConnected) { delete remove.dataset.sure; remove.textContent = "Delete this translation"; } }, 5000);
        return;
      }
      if (reading?.button === page.listen) stopReading();
      try {
        await invoke("delete_translation", { kind, id, language: page.language });
        page.stack.panels.delete(page.language);
        holder.remove();
        refreshAnalyses(); // So Translate offers the language again.
      } catch (e) { showError(holder, e); }
    });
    parts.push(rows, el("div", { class: "analysis-foot translation-foot" },
      el("span", {}, `Machine translation${data.engine ? ` with ${data.engine}` : ""}. Check it against the original; click a line to correct it.`), remove));
  } else if (page.open && busy) {
    parts.push(el("p", { class: "hint translation-intro" }, `The ${name} translation appears here when it's done. You can keep working meanwhile.`));
  } else if (page.open) {
    parts.push(el("p", { class: "hint translation-intro" },
      `Make a ${name} version of this ${kind} to read, correct, export or listen to. Your analysis model translates it line by line; names and words in your vocabulary stay as they are.`));
  }
  holder.replaceChildren(...parts);
  drawReadingStatus(page);
  markSpeaking(page);
}
