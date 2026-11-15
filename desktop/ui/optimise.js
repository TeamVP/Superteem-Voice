"use strict";

/// Local AI: one page showing everything the models do here and how fast, the one-button flow
/// that optimises them for this device (languages, then the plan, then downloading and trying it), the
/// callout at the top of Settings, and the suggestion on the home page once a note and a meeting are made.

icon.spark = '<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"></path><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"></path>';

const bytes = (n) => (n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.max(1, Math.round(n / 1e6))} MB`);

/// The plan and the state of things, fetched fresh.
const optimiseData = () => invoke("optimise_plan");

/// Optimising tells the page how it's going.
let optimisePage = null;
listen("optimise-progress", (e) => {
  if (optimisePage?.isConnected) drawOptimiseProgress(optimisePage, e.payload);
});

// ---------- Settings: the callout and the row

/// For the top of Settings: a callout while optimising would change something, else nothing.
async function optimiseCallout() {
  const data = await optimiseData().catch(() => null);
  if (!data) return null;
  const todo = data.plan.steps.filter((s) => !s.done && !s.kind.startsWith("remove"));
  if (!data.progress.running && (!todo.length || !data.stale)) return null;
  return el("div", { class: "optimise-callout" }, svg("spark", 22),
    el("div", { class: "grow" }, el("strong", {}, data.progress.running ? tr("Optimising Voice-OS…") : tr("Make Voice-OS faster and smarter on this {0}", { "0": deviceWord() })),
      el("small", {}, data.progress.running ? data.progress.step : optimiseSummary(todo))),
    el("button", { class: "primary pill", onclick: () => (data.progress.running ? showAiSettings() : showOptimise(OPTIMISE_FROM.settings)) }, data.progress.running ? tr("See progress") : tr("Optimise")));
}

/// "A better speech model, the graphics processor and a Spanish voice."
function optimiseSummary(todo) {
  const parts = [];
  if (todo.some((s) => s.kind === "gpu")) parts.push(tr("the GPU"));
  if (todo.some((s) => s.kind === "speech")) parts.push(tr("a better speech model"));
  if (todo.some((s) => s.kind === "analysis")) parts.push(tr("a better analysis model"));
  const voices = todo.filter((s) => s.kind === "voice").map((s) => s.title.replace(/^Reading (.+?):.*/, "$1"));
  if (voices.length) parts.push(`${voices.length === 1 ? "a" : ""} natural ${voices.join(" and ")} voice${voices.length > 1 ? "s" : ""}`);
  if (!parts.length) return tr("Check what runs here and how fast.");
  const list = parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts.at(-1)}` : parts[0];
  return tr("One click sets up {list}, chosen for this device.", { list: list });
}

// ---------- The home page suggestion

/// Shown once a note and a meeting have been made, when optimising would change something. "Not now"
/// stops it; Settings still offers it.
async function optimiseNudge() {
  const data = await optimiseData().catch(() => null);
  if (!data?.suggest || data.progress.running) return null;
  const todo = data.plan.steps.filter((s) => !s.done && !s.kind.startsWith("remove"));
  const card = el("div", { class: "optimise-nudge", role: "region", "aria-label": tr("Optimise for this device") },
    svg("spark", 22),
    el("div", { class: "grow" }, el("strong", {}, tr("Voice-OS can run better on this {0}", { "0": deviceWord() })), el("small", {}, optimiseSummary(todo))),
    el("div", { class: "optimise-nudge-actions" },
      el("button", { class: "primary pill", onclick: () => showOptimise(OPTIMISE_FROM.home) }, tr("Optimise")),
      el("button", { class: "link", onclick: async () => { await invoke("optimise_dismiss").catch(() => {}); card.remove(); } }, tr("Not now"))));
  return card;
}

// ---------- Settings › Local AI

async function showAiSettings(message) {
  leave();
  showPage();
  view = { kind: "ai-settings" };
  refreshItems();
  const page = el("div", { class: "page ai-page" }, backLinks(tr("‹ Settings"), () => showSettings()), el("h1", {}, tr("Local AI")));
  main.replaceChildren(page);
  const data = await optimiseData().catch((e) => { showError(page, e); return null; });
  if (!data) return;
  const { plan, current, last, progress } = data;
  const d = plan.profile;
  page.append(el("p", { class: "hint" }, tr("All of Voice-OS's AI runs here, offline: speech to text, analyses, translations and reading aloud. This is what each uses, and how fast.")));
  if (message) page.append(el("div", { class: "notice" }, message));

  if (progress.running) {
    optimisePage = el("div", { class: "group optimise-progress" });
    page.append(optimisePage);
    drawOptimiseProgress(optimisePage, progress);
  } else {
    const todo = plan.steps.filter((s) => !s.done && !s.kind.startsWith("remove"));
    // Never done; something new to recommend; set up as the person chose; or fully optimised.
    const [title, detail] = !last ? [tr("Not optimised yet"), optimiseSummary(todo)]
      : data.stale ? [tr("Something can be better"), optimiseSummary(todo)]
      : todo.length ? [tr("Set up your way"), tr("You kept your own choices. Recommendations are still here. Last checked {0}.", { "0": timeText(last.at) })]
      : [tr("Optimised for this device"), tr("Nothing to change. Last done {0}.", { "0": timeText(last.at) })];
    const label = !last || data.stale ? tr("Optimise") : todo.length ? tr("See recommendations") : tr("Optimise again");
    page.append(el("div", { class: "optimise-callout" }, svg("spark", 22),
      el("div", { class: "grow" }, el("strong", {}, title), el("small", {}, detail)),
      el("button", { class: !last || data.stale ? "primary pill" : "pill", onclick: () => showOptimise(OPTIMISE_FROM.ai) }, label)));
  }

  // The device.
  const gpuText = d.gpu ? `${d.gpu}${d.gpu_memory_gb ? `, ${d.gpu_memory_gb.toFixed(0)} GB` : d.integrated ? tr(", shares the computer's memory") : ""}` : tr("None usable");
  page.append(el("h2", {}, tr("This device")), el("div", { class: "group" },
    infoRow(tr("GPU"), d.gpu ? (current.gpu_on ? tr("Models run on it.") : tr("Off: models run on the CPU.")) : plan.notes.find((n) => n.includes("graphics")) ?? "", gpuText),
    infoRow(tr("Memory"), tr("Decides how large a model runs well."), `${d.ram_gb.toFixed(0)} GB`),
    infoRow(tr("Free space"), tr("Where models are kept."), d.free < 1e15 ? bytes(d.free) : "—"),
    infoRow(tr("Model storage"), tr("Speech, analysis and voice models."), bytes(current.storage))));

  // Each job.
  const where = (gpu) => (gpu ? tr("on the GPU") : tr("on the CPU"));
  const speechSpeed = last?.speech_s != null ? tr("10 s of speech in {0} s, {1}", { "0": last.speech_s.toFixed(1), "1": where(last.speech_on_gpu) }) : tr("Speed is measured when you optimise.");
  const words = last?.words_per_s != null ? tr("About {0} words a second, {1}", { "0": Math.round(last.words_per_s), "1": where(last.analysis_on_gpu) }) : tr("Speed is measured when you optimise.");
  page.append(el("h2", {}, tr("What runs here")), el("div", { class: "group" },
    infoRow(tr("Speech to text"), tr("{speechSpeed}. Dictation, commands and transcripts.", { speechSpeed: speechSpeed }), current.speech_ready ? current.speech : tr("{speech} (not downloaded)", { speech: current.speech })),
    infoRow(tr("Analysis"), current.server ? tr("On your server, {server}.", { server: current.server }) : tr("{words}. Summaries, action items and templates.", { words: words }), current.server ? tr("Server") : current.analysis ?? tr("None yet")),
    infoRow(tr("Translation"), tr("Made by the analysis model, line by line."), current.server ? tr("Server") : current.analysis ?? tr("None yet")),
    infoRow(tr("Reading aloud"), tr("Notes, transcripts, analyses and translations."), current.languages.map(([, v]) => v).filter(Boolean)[0] ?? tr("System voice"))));

  // Each language.
  const quality = { good: tr("Transcribed well"), fair: tr("Transcribes fairly well"), weak: tr("Rough transcription"), none: tr("Can't transcribe yet") };
  page.append(el("h2", {}, tr("Your languages")), el("div", { class: "group" }, current.languages.map(([code, voice, q, direct]) =>
    infoRow(languageName(code), `${quality[q] ?? ""} · ${direct ? tr("Analyses in it") : tr("Analysed via English")}`, voice ?? tr("System voice")))));

  // Things to tidy.
  const removable = plan.steps.filter((s) => s.kind.startsWith("remove"));
  if (removable.length) {
    page.append(el("h2", {}, tr("No longer needed")), el("div", { class: "group" }, removable.map((s) =>
      infoRow(s.title.replace(/^Remove /, ""), s.reason, bytes(s.frees)))),
    el("p", { class: "hint" }, tr("Optimising removes these, or remove them on the pages below.")));
  }
  if (plan.notes.length) page.append(el("ul", { class: "optimise-notes" }, plan.notes.map((n) => el("li", {}, n))));

  page.append(el("h2", {}, tr("Choose for yourself")), el("div", { class: "group" },
    linkRow(tr("Analysis"), tr("Model, GPU and local server"), () => { analysisOrigin = null; showAnalysisSettings(); }),
    linkRow(tr("Languages"), tr("Your languages and speech models"), () => showLanguages()),
    linkRow(tr("Reading aloud"), tr("Voices per language, speed"), () => showReadingSettings())));
}

function infoRow(label, detail, value) {
  return el("div", { class: "row" }, el("span", { class: "label" }, label, detail ? el("small", {}, detail) : null), el("span", { class: "value" }, value ?? ""));
}

// ---------- The flow

/// Where optimising was started from, to go back to: [label, show].
const OPTIMISE_FROM = {
  settings: [tr("Settings"), () => showSettings()],
  ai: [tr("Local AI"), () => showAiSettings()],
  home: ["Home", () => showHome()],
};
let optimiseFrom = OPTIMISE_FROM.settings;

/// Optimising, its own branch from Settings: first the languages, then the plan to confirm, then it runs
/// and the AI page shows how it went.
async function showOptimise(from) {
  if (from) optimiseFrom = from;
  leave();
  showPage();
  view = { kind: "optimise" };
  refreshItems();
  // Straight from the app, not the stand-in used when it can't be read, so saving can't drop a language.
  let info;
  try { info = await invoke("get_languages"); languageInfo = info; } catch (e) {
    main.replaceChildren(el("div", { class: "page" }, backLinks(`‹ ${optimiseFrom[0]}`, optimiseFrom[1]), el("div", { class: "error" }, tr("Your languages couldn't be read: {e}", { e: e }))));
    return;
  }
  const page = el("div", { class: "page optimise-page" }, backLinks(`‹ ${optimiseFrom[0]}`, optimiseFrom[1]),
    el("div", { class: "optimise-step" }, tr("Step 1 of 2 · Your languages")), el("h1", {}, tr("Optimise for this {0}", { "0": deviceWord() })),
    el("p", { class: "hint" }, tr("First, the languages you speak and read. Voice-OS then picks the best speech model, analysis model and voices for them.")));
  const chosen = new Set(info.chosen);
  const list = el("div", { class: "group" });
  const draw = () => {
    list.replaceChildren(...[...chosen].map((code) => {
      const box = el("input", { type: "checkbox", checked: true, disabled: code === info.primary, "aria-label": languageName(code) });
      box.addEventListener("change", () => { chosen.delete(code); draw(); });
      return el("label", { class: "row" }, box, el("span", { class: "label" }, languageName(code), code === info.primary ? el("small", {}, tr("Your main language")) : null));
    }));
    const add = el("select", { "aria-label": tr("Add a language") }, el("option", { value: "" }, tr("Add a language…")),
      (info.catalogue ?? []).filter((l) => !chosen.has(l.code)).map((l) => el("option", { value: l.code }, l.native && l.native !== l.name ? `${l.name} · ${l.native}` : l.name)));
    add.addEventListener("change", () => { if (add.value) { chosen.add(add.value); draw(); } });
    list.append(el("div", { class: "row add-language" }, add));
  };
  draw();
  const next = el("button", { class: "primary pill" }, tr("Next: the plan"));
  next.addEventListener("click", async () => {
    next.disabled = true;
    try {
      // Saved only when changed here.
      const same = chosen.size === info.chosen.length && info.chosen.every((c) => chosen.has(c));
      if (!same) await invoke("save_languages", { languages: [...chosen], primary: info.primary });
      await languageData(true);
      showOptimisePlan();
    } catch (e) {
      next.disabled = false;
      showError(page, e);
    }
  });
  page.append(el("h2", {}, tr("Your languages")), list, el("div", { class: "toolbar" }, el("span", { class: "grow" }), next));
  main.replaceChildren(page);
}

async function showOptimisePlan() {
  leave();
  showPage();
  view = { kind: "optimise" };
  const page = el("div", { class: "page optimise-page" }, backLinks(`‹ ${optimiseFrom[0]}`, optimiseFrom[1]),
    el("div", { class: "optimise-step" }, tr("Step 2 of 2 · The plan · "), el("button", { class: "link inline", onclick: () => showOptimise() }, tr("Change languages"))),
    el("h1", {}, tr("The plan for this {0}", { "0": deviceWord() })));
  page.append(el("p", { class: "hint" }, el("span", { class: "spinner" }), tr(" Looking at this device…")));
  main.replaceChildren(page);
  const data = await optimiseData().catch((e) => { showError(page, e); return null; });
  if (!data) return;
  page.querySelector(".hint").remove();
  const { plan } = data;
  const d = plan.profile;
  page.append(el("p", { class: "hint" }, tr("{0} · {1} GB memory · {2} free. Untick anything you don't want.", { "0": d.gpu ?? tr("No usable GPU"), "1": d.ram_gb.toFixed(0), "2": d.free < 1e15 ? bytes(d.free) : "plenty of" })));
  const picked = new Map();
  const total = el("strong", {});
  const go = el("button", { class: "primary pill" });
  const sum = () => {
    const steps = plan.steps.filter((s) => picked.get(`${s.kind}:${s.id}`));
    const download = steps.reduce((n, s) => n + s.download, 0);
    const frees = steps.reduce((n, s) => n + s.frees, 0);
    total.textContent = download ? tr("Downloads {download}{1}", { download: bytes(download), "1": frees ? `, frees ${bytes(frees)}` : "" }) : frees ? tr("Frees {frees}", { frees: bytes(frees) }) : steps.length ? tr("Nothing to download") : tr("Nothing to change");
    go.textContent = download ? tr("Download and set up ({download})", { download: bytes(download) }) : tr("Set up");
    go.disabled = !steps.length;
  };
  const group = el("div", { class: "group optimise-plan" });
  const kept = new Set(data.last?.kept ?? []);
  for (const s of plan.steps) {
    const key = `${s.kind}:${s.id}`;
    const text = el("span", { class: "label" }, s.title, el("small", {}, s.reason));
    // Done: nothing to choose; a green tick and "Done".
    if (s.done) {
      group.append(el("div", { class: "row done" }, el("span", { class: "plan-tick", role: "img", "aria-label": tr("Done") }, svg("check", 18)), text,
        el("span", { class: "plan-done" }, tr("Done"))));
      continue;
    }
    // Ticked, unless the person chose to go without it before.
    picked.set(key, !kept.has(key));
    const box = el("input", { type: "checkbox", checked: !kept.has(key), "aria-label": s.title });
    box.addEventListener("change", () => { picked.set(key, box.checked); sum(); });
    group.append(el("label", { class: "row" }, box, text,
      el("span", { class: "value" }, s.download ? bytes(s.download) : s.frees ? tr("Frees {0}", { "0": bytes(s.frees) }) : "")));
  }
  page.append(group);
  if (plan.notes.length) page.append(el("ul", { class: "optimise-notes" }, plan.notes.map((n) => el("li", {}, n))));
  page.append(el("p", { class: "hint" }, tr("Each model is tried once in place, so you see how fast it runs. Downloads continue if you leave.")));
  go.addEventListener("click", async () => {
    const steps = plan.steps.filter((s) => picked.get(`${s.kind}:${s.id}`)).map((s) => [s.kind, s.id]);
    go.disabled = true;
    try {
      await invoke("optimise_apply", { steps });
      showAiSettings();
    } catch (e) {
      go.disabled = false;
      showError(page, e);
    }
  });
  // Happy as things are: what's left becomes the person's choice, and isn't suggested again until
  // something changes. They can come back and follow the recommendations whenever they like.
  const keep = plan.steps.some((s) => !s.done)
    ? el("button", { class: "pill", title: tr("Keep things as they are. Voice-OS stops suggesting these until something changes."), onclick: async () => {
      try {
        await invoke("optimise_keep");
        showAiSettings(tr("Kept as is. Voice-OS won't suggest these unless something changes. Come back any time."));
      } catch (e) { showError(page, e); }
    } }, tr("Keep my setup"))
    : null;
  page.append(el("div", { class: "toolbar optimise-go" }, total, el("span", { class: "grow" }), el("button", { class: "pill", onclick: optimiseFrom[1] }, tr("Cancel")), keep, go));
  sum();
  main.replaceChildren(page);
}

function drawOptimiseProgress(holder, p) {
  if (!p.running) {
    if (p.finished) {
      const problems = p.problems ?? [];
      const message = p.error ?? (problems.length ? tr("Optimised, except: {0}", { "0": problems.join(" · ") }) : tr("Optimised. Everything below is set up and tested here."));
      optimisePage = null;
      if (view.kind === "ai-settings") showAiSettings(message);
    }
    return;
  }
  const bar = el("div", { class: "optimise-bar" }, el("span", { style: `width:${Math.round((p.fraction ?? 0) * 100)}%` }));
  holder.replaceChildren(el("div", { class: "row" }, el("span", { class: "spinner" }),
    el("span", { class: "label" }, p.step || tr("Starting…"), el("small", {}, tr("Step {index} of {count}{2}", { index: p.index, count: p.count, "2": p.fraction > 0 && p.fraction < 1 ? ` · ${Math.round(p.fraction * 100)}%` : "" })), bar),
    el("button", { class: "pill", onclick: () => invoke("optimise_cancel") }, tr("Stop"))));
}
