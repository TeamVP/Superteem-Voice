"use strict";

// ---------- The app's own words, in the language the person picked ----------
//
// English is written in the code and is the key: tr("Save") looks "Save" up in I18N_STRINGS (strings.js),
// whose rows hold one column per language in UI_LANGS order after English. A word with no translation
// shows in English. Placeholders are {name}: tr("Updated to {version}", { version }).

const UI_LANGS = [
  ["en", "English"], ["uk", "Українська"], ["es", "Español"], ["zh-CN", "简体中文"], ["hi", "हिन्दी"], ["ar", "العربية"],
  ["bn", "বাংলা"], ["fr", "Français"], ["id", "Bahasa Indonesia"], ["pt-BR", "Português (Brasil)"], ["ja", "日本語"],
];

/// The language chosen on this device. It's read before anything draws, so words made once, as the
/// scripts load, are already in it; changing it reloads the page.
const UI_LANG = (() => {
  try {
    const saved = localStorage.getItem("uiLang");
    if (UI_LANGS.some(([code]) => code === saved)) return saved;
  } catch {}
  return "en";
})();
const UI_COLUMN = UI_LANGS.findIndex(([code]) => code === UI_LANG) - 1;
document.documentElement.lang = UI_LANG;
// Arabic reads right to left: the layout mirrors, while timelines and recordings keep running left to right.
const UI_RTL = UI_LANG === "ar";
document.documentElement.dir = UI_RTL ? "rtl" : "ltr";
/// Dates and times follow the chosen language; in English, the device's own way. Numbers stay in the
/// digits 0–9 everywhere, so a date and a count beside it read alike.
const UI_LOCALE = UI_LANG === "en" ? undefined : ["ar", "bn"].includes(UI_LANG) ? UI_LANG + "-u-nu-latn" : UI_LANG;

const trFill = (text, vars) => vars ? text.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k] ?? "") : m)) : text;

function tr(text, vars) {
  const cell = UI_COLUMN >= 0 ? I18N_STRINGS[text]?.[UI_COLUMN] : null;
  return trFill(typeof cell === "string" && cell ? cell : text, vars);
}

/// A count's words: `one` and `other` are English, and the row for `other` holds each language's forms,
/// as { one, few, many, other } where its grammar needs them. `{n}` is the count.
function trn(n, one, other, vars) {
  const all = { n, ...vars };
  const cell = UI_COLUMN >= 0 ? I18N_STRINGS[other]?.[UI_COLUMN] : null;
  if (typeof cell === "string" && cell) return trFill(cell, all);
  if (cell && typeof cell === "object") {
    const form = new Intl.PluralRules(UI_LANG).select(n);
    return trFill(cell[form] ?? cell.other, all);
  }
  return trFill(n === 1 ? one : other, all);
}

/// "A and B", or "A, B and C" in the chosen language's own way.
function trList(items) {
  if (UI_LOCALE) {
    try {
      return new Intl.ListFormat(UI_LOCALE, { type: "conjunction" }).format(items.map(String));
    } catch {}
  }
  return items.join(" and ");
}

/// A language's name in the chosen language, such as "іспанська" for es; null when it isn't known there.
function trLanguageName(tag) {
  if (!UI_LOCALE) return null;
  try {
    const name = new Intl.DisplayNames([UI_LOCALE], { type: "language", fallback: "none" }).of(tag);
    return name ? name[0].toLocaleUpperCase(UI_LOCALE) + name.slice(1) : null;
  } catch {
    return null;
  }
}

/// `text` in one of the app's languages other than the chosen one: a question asked in the language
/// just picked, before the page is drawn again in it.
function trIn(code, text, vars) {
  const column = UI_LANGS.findIndex(([c]) => c === code) - 1;
  const cell = column >= 0 ? I18N_STRINGS[text]?.[column] : null;
  return trFill(typeof cell === "string" && cell ? cell : text, vars);
}

/// The spoken language that matches each of the app's: 简体中文 is Mandarin in simplified characters, and
/// Português (Brasil) is Portuguese.
const UI_SPOKEN = { en: "en", uk: "uk", es: "es", "zh-CN": "zh", hi: "hi", ar: "ar", bn: "bn", fr: "fr", id: "id", "pt-BR": "pt", ja: "ja" };
const uiLangFor = (spoken) => Object.keys(UI_SPOKEN).find((code) => UI_SPOKEN[code] === spoken) ?? null;

/// Offers to match the app's language and the spoken one that the person turned down, such as
/// "ui:uk:en" (show the app in Ukrainian instead of English), so the same one isn't offered again.
function langOfferDeclined(pair) {
  try {
    return JSON.parse(localStorage.getItem("langOffersDeclined") ?? "[]").includes(pair);
  } catch {
    return false;
  }
}

function declineLangOffer(pair) {
  try {
    const all = JSON.parse(localStorage.getItem("langOffersDeclined") ?? "[]");
    if (!all.includes(pair)) localStorage.setItem("langOffersDeclined", JSON.stringify([...all, pair]));
  } catch {}
}

/// Saves the language on this device and in its preferences, then draws everything again in it,
/// coming back to `reopen` ("settings" or "languages").
function pickUiLang(code, reopen = "settings") {
  try {
    localStorage.setItem("uiLang", code);
    sessionStorage.setItem("reopen", reopen);
  } catch {}
  window.__TAURI__.core.invoke("save_prefs", { uiLang: code }).catch(() => {}).finally(() => location.reload());
}

/// The words written into index.html, put into the chosen language once.
(() => {
  if (UI_COLUMN < 0) return;
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.nodeValue.trim();
    // The product's name stays as it is.
    if (text && !node.parentElement.closest(".brand")) node.nodeValue = node.nodeValue.replace(text, tr(text));
  }
  for (const attr of ["title", "aria-label", "placeholder"]) {
    for (const node of document.querySelectorAll(`[${attr}]`)) node.setAttribute(attr, tr(node.getAttribute(attr)));
  }
})();
