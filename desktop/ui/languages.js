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

/// The picker beside a note's or meeting's status: its main language, and a second one for a note or
/// meeting where people switch between two. `onChange(list)` saves and says whether it worked.
function languagePicker(current, onChange) {
  let saved = [...current];
  const wrap = el("span", { class: "language-picker-wrap", title: "The languages this is in" });
  const main = el("select", { class: "language-picker", "aria-label": "Language" }, languageOptions(saved[0]));
  const second = el("select", { class: "language-picker second", "aria-label": "Second language" },
    languageOptions(saved[1] ?? "", { none: saved[1] ? "Only one language" : "+ Second language" }));
  const draw = () => {
    main.replaceChildren(...languageOptions(saved[0]));
    second.replaceChildren(...languageOptions(saved[1] ?? "", { none: saved[1] ? "Only one language" : "+ Second language" }).filter(Boolean));
    for (const opt of second.querySelectorAll("option")) opt.disabled = opt.value === saved[0];
  };
  const change = async () => {
    const list = [main.value, second.value].filter((c, i, a) => c && a.indexOf(c) === i);
    if (await onChange(list)) saved = list;
    draw();
  };
  main.addEventListener("change", change);
  second.addEventListener("change", change);
  draw();
  wrap.append(svg("globe", 14), main, second);
  return wrap;
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
  group.append(el("div", { class: "row" }, add));
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

  // Speech models.
  const list = el("div", { class: "group" });
  for (const m of models) {
    const status = el("small", {}, m.about);
    const row = el("div", { class: "row" }, el("span", { class: "label" }, m.name, m.suggested ? el("span", { class: "badge" }, "Suits this device") : null, status));
    if (m.ready) {
      row.append(el("span", { class: "badge ok" }, "Downloaded"));
      if (m.multilingual) {
        row.append(el("button", { class: "link", onclick: async () => {
          try { await invoke("delete_speech_model", { id: m.id }); showLanguages(); } catch (e) { showError(page, e); }
        } }, "Delete"));
      }
    } else {
      const get = el("button", { class: "pill" }, `Download (${sizeText(m.size)})`);
      get.addEventListener("click", async () => {
        get.disabled = true;
        const off = await listen("speech-model-download", (e) => { if (e.payload.id === m.id) status.textContent = `Downloading… ${Math.round(e.payload.fraction * 100)}%`; });
        try { await invoke("download_speech_model", { id: m.id }); showLanguages(); } catch (e) { get.disabled = false; status.textContent = String(e); } finally { off(); }
      });
      row.append(get);
    }
    list.append(row);
  }
  const multi = models.filter((m) => m.multilingual && m.ready);
  if (multi.length > 1) {
    const pick = el("select", { "aria-label": "Speech model for other languages" }, el("option", { value: "" }, "The best one downloaded"),
      multi.map((m) => el("option", { value: m.id, selected: data.speech_model === m.id }, m.name)));
    pick.addEventListener("change", () => settings({ speechModel: pick.value }));
    list.append(el("div", { class: "row" }, el("span", { class: "label" }, "For languages other than English"), pick));
  }
  page.append(el("h2", {}, "Speech models"), list,
    el("p", { class: "hint" }, "English uses the English model. Other languages need a multilingual one, which you're offered when you first choose such a language. "
      + "Spoken commands such as “Hey Superteem” and “bookmark that” are English for now."));
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

/// The translation panel on the page now: { kind, id, holder, language, original, render }.
let translationPage = null;

/// A section showing a meeting's or note's translation into one of the person's languages, line by line,
/// each correctable, with the original beside it. `languages` are the item's; `ctx.go(line)` jumps to a
/// line in the original.
function translationPanel(kind, id, languages, ctx = {}) {
  const holder = el("section", { class: "translation", "aria-label": "Translation" });
  const targets = () => (languageInfo?.chosen ?? []).filter((c) => !(languages.length === 1 && languages[0] === c));
  const remembered = (() => { try { return localStorage.getItem(`translation:${kind}:${id}`); } catch { return null; } })();
  const page = { kind, id, holder, languages, ctx, open: !!remembered, language: remembered || (languageInfo?.translate_into ?? []).find((c) => targets().includes(c)) || targets()[0] };
  translationPage = page;
  if (!page.language) return null;
  drawTranslation(page);
  return holder;
}

/// Called by the job events: redraws the panel when its translation moves on or finishes.
function refreshTranslation(job, finished) {
  const page = translationPage;
  if (!page || !page.holder.isConnected || job.kind !== page.kind || job.id !== page.id || job.template !== `translate:${page.language}`) return;
  if (finished) {
    page.progress = null;
    page.error = job.error ?? null;
    page.open = true;
  } else {
    page.progress = job.parts > 1 ? `Translating: part ${job.part} of ${job.parts}…` : "Translating…";
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
  if (translationPage !== page) return;
  const queue = await invoke("get_analyses", { kind, id }).then((d) => d.queue).catch(() => ({ current: null, waiting: [] }));
  const job = `translate:${page.language}`;
  const running = queue.current?.kind === kind && queue.current?.id === id && queue.current?.template === job;
  const waiting = queue.waiting.some((j) => j.kind === kind && j.id === id && j.template === job);
  const translated = data.lines.length - data.missing;
  const pick = el("select", { class: "language-picker", "aria-label": "Translate into" },
    (languageInfo?.chosen ?? []).map((c) => el("option", { value: c, selected: c === page.language }, languageName(c))),
    el("optgroup", { label: "More languages" }, (languageInfo?.catalogue ?? []).filter((l) => !(languageInfo?.chosen ?? []).includes(l.code))
      .map((l) => el("option", { value: l.code, selected: l.code === page.language }, l.name))));
  pick.addEventListener("change", () => {
    page.language = pick.value;
    page.open = true;
    try { localStorage.setItem(`translation:${kind}:${id}`, pick.value); } catch {}
    drawTranslation(page);
  });
  const go = el("button", { class: "pill", disabled: running || waiting }, data.missing === data.lines.length ? "Translate" : data.missing ? `Translate ${data.missing} new line${data.missing > 1 ? "s" : ""}` : "Up to date");
  if (!data.missing) go.disabled = true;
  go.addEventListener("click", async () => {
    page.error = null;
    try { await invoke("translate_item", { kind, id, language: page.language }); page.progress = "Waiting its turn"; drawTranslation(page); } catch (e) { page.error = String(e); drawTranslation(page); }
  });
  const cancel = el("button", { class: "link", onclick: () => invoke("cancel_analysis", { kind, id, template: job }) }, "Cancel");
  const exporter = exportMenu([["txt", "Translation as text", ".txt"], ["md", "Translation as Markdown", ".md"]],
    (format) => invoke("export_translation", { kind, id, language: page.language, format }));
  const toggle = el("button", { class: "icon-only chevron", "aria-expanded": String(page.open), title: page.open ? "Hide" : "Show",
    onclick: () => { page.open = !page.open; drawTranslation(page); } }, svg(page.open ? "down" : "chevron", 16));
  const head = el("div", { class: "analysis-head translation-head" }, toggle, svg("globe", 16), el("strong", {}, "Translation"), pick,
    running || waiting ? el("span", { class: "badge busy" }, el("span", { class: "spinner" }), running ? page.progress ?? "Loading the model…" : "Waiting its turn") : null,
    !running && !waiting && translated ? el("span", { class: "hint" }, data.missing ? `${translated} of ${data.lines.length} lines` : "") : null,
    el("span", { class: "grow" }), running || waiting ? cancel : go, translated ? exporter : null);
  const parts = [head];
  if (translated) parts.push(exporter.status);
  if (page.error) parts.push(el("p", { class: "danger" }, page.error));
  if (data.advice) parts.push(el("p", { class: "hint" }, data.advice, " ", el("button", { class: "link inline", onclick: () => { analysisOrigin = { kind, id }; showAnalysisSettings(); } }, "Analysis settings")));
  if (page.open && translated) {
    const rows = el("div", { class: "translation-lines" });
    for (const line of data.lines) {
      const text = line.text ? editable(line.text, async (value) => {
        try { await invoke("edit_translation", { kind, id, language: page.language, line: line.id, text: value }); drawTranslation(page); } catch (e) { showError(holder, e); }
      }, "div") : el("div", { class: "hint" }, "Not translated yet");
      setLanguage([text], page.language);
      const original = el("div", { class: "translation-original" }, line.original);
      setLanguage([original], page.languages[0]);
      const label = line.at_s != null ? el("button", { class: "link mono", title: "Go to this line", onclick: () => page.ctx.go?.(line) }, clock(line.at_s)) : null;
      const flags = [];
      if (line.edited) flags.push(el("span", { class: "badge", title: "Corrected by you; translating again keeps it. Clear the text to go back to the machine translation." }, "Corrected"));
      if (line.original_changed) flags.push(el("span", { class: "badge stale", title: "The original line changed after you corrected this" }, "Original changed"));
      if (line.warning) flags.push(el("span", { class: "badge stale", title: line.warning }, "Check"));
      rows.append(el("div", { class: `translation-line${line.same ? " same" : ""}` },
        line.speaker || label ? el("div", { class: "translation-who" }, label, line.speaker ? el("strong", {}, line.speaker) : null, ...flags) : flags.length ? el("div", { class: "translation-who" }, ...flags) : null,
        text, line.same ? null : original));
    }
    parts.push(rows, el("p", { class: "analysis-foot" }, `Machine translation${data.engine ? ` with ${data.engine}` : ""}. Check it against the original. Click a line to correct it.`));
  } else if (page.open && !running && !waiting) {
    parts.push(el("p", { class: "hint" }, `Translate this ${kind} into ${languageName(page.language)} with the analysis language model, line by line. Lines already in ${languageName(page.language)} are kept as they are.`));
  }
  holder.replaceChildren(...parts);
}
