// The note editor: TipTap with a small toolbar, Markdown in and out, pictures and files kept with the
// note, and places held for dictation. Built into ui/vendor/note-editor.js by `npm run build:editor`
// (the built file is checked in, so building the app needs no JavaScript tooling).
//
// The page gives it a `host`:
//   addFile(File) -> Promise<{ file, name, image }>   keeps a file with the note
//   importImage(url) -> Promise<{ file }>             fetches a picture from the web into the note
//   resolve(file) -> string                           the address a kept file is shown from
//   open(href)                                        opens a link or a kept file
//   onChange()                                        the text changed
//   notify(text)                                      a short message for the person

import { Editor, Extension } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import { TaskItem, TaskList } from "@tiptap/extension-list";
import { TableKit } from "@tiptap/extension-table";
import TextAlign from "@tiptap/extension-text-align";
import { Placeholder } from "@tiptap/extensions";
import { Markdown } from "@tiptap/markdown";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

const ATTACHMENT = "attachment:";
const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
const MOD = isMac ? "⌘" : "Ctrl";
const keys = (text) => text.replace("Mod", MOD).replace(/\+/g, isMac ? "" : "+").replace("Shift", isMac ? "⇧" : "Shift").replace("Alt", isMac ? "⌥" : "Alt");

const ICONS = {
  undo: '<path d="M9 14L4 9l5-5"></path><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"></path>',
  redo: '<path d="M15 14l5-5-5-5"></path><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13"></path>',
  bold: '<path d="M6 12h9a4 4 0 0 1 0 8H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h7a4 4 0 0 1 0 8"></path>',
  italic: '<path d="M19 4h-9M14 20H5M15 4L9 20"></path>',
  h1: '<path d="M4 12h8M4 18V6M12 18V6M17 12l3-2v8"></path>',
  h2: '<path d="M4 12h8M4 18V6M12 18V6M21 18h-4c0-4 4-3 4-6 0-1.5-2-2.5-4-1"></path>',
  bullets: '<path d="M9 6h12M9 12h12M9 18h12M4 6h.01M4 12h.01M4 18h.01"></path>',
  numbers: '<path d="M10 6h11M10 12h11M10 18h11M4 6h1v4M4 10h2M6 18H4c0-1 2-2 2-3s-1-1.5-2-1"></path>',
  plus: '<path d="M12 5v14M5 12h14"></path>',
  more: '<circle cx="5" cy="12" r="1.4" fill="currentColor"></circle><circle cx="12" cy="12" r="1.4" fill="currentColor"></circle><circle cx="19" cy="12" r="1.4" fill="currentColor"></circle>',
  align: '<path d="M4 6h16M4 12h10M4 18h13"></path>',
  "align-left": '<path d="M4 6h16M4 12h10M4 18h13"></path>',
  "align-center": '<path d="M4 6h16M7 12h10M5.5 18h13"></path>',
  "align-right": '<path d="M4 6h16M10 12h10M7 18h13"></path>',
  link: '<path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7"></path><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"></path>',
  strike: '<path d="M16 4H9a3 3 0 0 0-2.8 4M14 12a4 4 0 0 1 0 8H6M4 12h16"></path>',
  todo: '<path d="M3 5l2 2 3-3M3 12l2 2 3-3M3 19l2 2 3-3M13 6h8M13 12h8M13 18h8"></path>',
  code: '<path d="M16 18l6-6-6-6M8 6l-6 6 6 6"></path>',
  paste: '<path d="M15 2H9a1 1 0 0 0-1 1v2a1 1 0 0 0 1 1h6a1 1 0 0 0 1-1V3a1 1 0 0 0-1-1z"></path><path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2h-2M9 12h6M12 12v6"></path>',
  copy: '<rect x="9" y="9" width="12" height="12" rx="2"></rect><path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1"></path>',
  keyboard: '<rect x="2.5" y="6" width="19" height="12" rx="2"></rect><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M6 14h.01M18 14h.01M9 14h6"></path>',
  image: '<rect x="3" y="3" width="18" height="18" rx="2"></rect><circle cx="9" cy="9" r="2"></circle><path d="M21 15l-3.1-3.1a2 2 0 0 0-2.8 0L6 21"></path>',
  file: '<path d="M21.4 11.1l-9.2 9.2a6 6 0 0 1-8.5-8.5l8.6-8.6a4 4 0 0 1 5.7 5.7l-8.6 8.6a2 2 0 0 1-2.8-2.8l8.5-8.5"></path>',
  globe: '<circle cx="12" cy="12" r="9"></circle><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"></path>',
  table: '<rect x="3" y="3" width="18" height="18" rx="2"></rect><path d="M3 9h18M3 15h18M9 3v18M15 3v18"></path>',
  rule: '<path d="M4 12h16"></path>',
  quote: '<path d="M7 7H4v6h3l-1 4M17 7h-3v6h3l-1 4"></path>',
};

function icon(name, size = 16) {
  const span = document.createElement("span");
  span.className = "ne-icon";
  span.setAttribute("aria-hidden", "true");
  span.innerHTML = `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;
  return span;
}

function h(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key === "class") node.className = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else if (value === true) node.setAttribute(key, "");
    else if (value !== false && value != null) node.setAttribute(key, value);
  }
  for (const child of children.flat()) if (child != null && child !== false) node.append(child.nodeType ? child : document.createTextNode(String(child)));
  return node;
}

/// Plain text, as notes were before this editor: each line a paragraph.
function plainToDoc(text) {
  const lines = String(text ?? "").replace(/\r\n?/g, "\n").split("\n");
  return { type: "doc", content: lines.map((line) => (line ? { type: "paragraph", content: [{ type: "text", text: line }] } : { type: "paragraph" })) };
}

// ---------- Dictation: places held in the text while speech is written down

const dictationKey = new PluginKey("dictation");

/// Slots are positions kept through every edit. Each clip's slot shows "Writing it down…" until its
/// words arrive; the anchor, which is not shown, marks where the next clip goes.
function dictationPlugin() {
  return new Plugin({
    key: dictationKey,
    state: {
      init: () => ({ slots: [], fades: DecorationSet.empty }),
      apply(tr, value) {
        const meta = tr.getMeta(dictationKey) ?? {};
        const filled = meta.fill != null ? value.slots.find((s) => s.id === meta.fill) : null;
        // Words written into a slot push the slots after it along; anything else typed at a slot goes after it.
        let slots = value.slots.map((s) => ({ ...s, pos: tr.mapping.map(s.pos, filled && s.order > filled.order ? 1 : -1) }));
        if (filled) slots = slots.filter((s) => s.id !== filled.id);
        if (meta.remove != null) slots = slots.filter((s) => s.id !== meta.remove);
        if (meta.clear) slots = [];
        if (meta.add) slots = [...slots, meta.add];
        let fades = value.fades.map(tr.mapping, tr.doc);
        if (meta.fade) fades = fades.add(tr.doc, [Decoration.inline(meta.fade.from, meta.fade.to, { class: "dictated" }, { fade: meta.fade.id })]);
        if (meta.unfade != null) fades = fades.remove(fades.find(undefined, undefined, (spec) => spec.fade === meta.unfade));
        return { slots, fades };
      },
    },
    props: {
      decorations(state) {
        const { slots, fades } = dictationKey.getState(state);
        const widgets = slots.filter((s) => !s.anchor).map((s) => Decoration.widget(s.pos, () =>
          h("span", { class: "dictation-slot", contenteditable: "false" }, h("span", { class: "spinner" }), "Writing it down…"), { side: -1000 + s.order, key: `slot-${s.id}` }));
        return widgets.length ? fades.add(state.doc, widgets) : fades;
      },
    },
  });
}

/// Words as a typist would put them between `before` and `after`: spaced, and capitalised to fit.
function fitWords(words, before, after) {
  words = words.trim();
  if (!words) return "";
  const sentenceStart = !before.trim() || /[.!?]["')\]]?\s*$/.test(before) || /\n\s*$/.test(before);
  if (sentenceStart) words = words[0].toUpperCase() + words.slice(1);
  else if (/^[A-Z][a-z]/.test(words) && !/^I\b/.test(words)) words = words[0].toLowerCase() + words.slice(1);
  // Whisper ends most phrases with a full stop, which reads wrongly mid-sentence.
  if (after.trim() && !/^[\s]*[A-Z\n]/.test(after)) words = words.replace(/[.]$/, "");
  const lead = before && !/\s$/.test(before) ? " " : "";
  const trail = after && !/^[\s.,;:!?)]/.test(after) ? " " : "";
  return lead + words + trail;
}

function dictationApi(editor) {
  let nextId = 1;
  let madeLine = false;
  const slots = () => dictationKey.getState(editor.state).slots;
  const find = (id) => slots().find((s) => s.id === id);
  const send = (tr, meta) => editor.view.dispatch(tr.setMeta(dictationKey, meta));
  return {
    /// Holds the cursor's place for a dictation, on a line of its own after a finished paragraph.
    place() {
      const { state } = editor;
      let tr = state.tr;
      let pos = state.selection.to;
      const $pos = state.doc.resolve(pos);
      madeLine = false;
      if ($pos.parent.isTextblock && $pos.parent.content.size > 0 && $pos.parentOffset === $pos.parent.content.size && $pos.parent.type.name !== "codeBlock") {
        tr = tr.split(pos);
        pos += 2;
        madeLine = true;
        tr = tr.setSelection(state.selection.constructor.near(tr.doc.resolve(pos)));
      }
      const id = nextId++;
      send(tr, { add: { id, pos, anchor: true, order: Number.MAX_SAFE_INTEGER } });
      return id;
    },
    /// A "Writing it down…" place for one clip, after the clips before it.
    clip(anchorId) {
      const anchor = find(anchorId);
      if (!anchor) return null;
      const id = nextId++;
      send(editor.state.tr, { add: { id, pos: anchor.pos, anchor: false, order: id } });
      editor.view.dom.querySelector(".dictation-slot:last-of-type")?.scrollIntoView({ block: "nearest" });
      return id;
    },
    /// Puts a clip's words where its place was. True when something was written.
    fill(id, words) {
      const slot = find(id);
      if (!slot) return false;
      const { doc } = editor.state;
      const text = fitWords(words ?? "", doc.textBetween(0, slot.pos, "\n", "\n"), doc.textBetween(slot.pos, doc.content.size, "\n", "\n"));
      if (!text) { send(editor.state.tr, { remove: id }); return false; }
      // Freshly dictated words show green and fade into the text.
      send(editor.state.tr.insertText(text, slot.pos), { fill: id, fade: { id, from: slot.pos, to: slot.pos + text.length } });
      setTimeout(() => { if (!editor.isDestroyed) send(editor.state.tr, { unfade: id }); }, 12000);
      return true;
    },
    remove(id) {
      if (find(id)) send(editor.state.tr, { remove: id });
    },
    /// The dictation is over: its marker goes, and so does the line made for it when nothing was said.
    end(anchorId, wrote) {
      const anchor = find(anchorId);
      if (!anchor) return;
      let tr = editor.state.tr;
      if (!wrote && madeLine) {
        const $at = tr.doc.resolve(anchor.pos);
        if ($at.parent.isTextblock && $at.parent.content.size === 0 && $at.depth > 0 && tr.doc.childCount > 1) tr = tr.delete($at.before(), $at.after());
      }
      send(tr, { clear: true });
    },
    pending: () => slots().some((s) => !s.anchor),
  };
}

// ---------- The editor

/// What a note can hold, and Markdown in and out. `files(file)` gives the address a kept file is shown from.
function noteExtensions(files) {
  const resolve = (src) => (src && src.startsWith(ATTACHMENT) ? files(src.slice(ATTACHMENT.length)) : src);
  const unresolve = (src) => {
    if (!src) return src;
    const base = files("");
    return base && src.startsWith(base) ? ATTACHMENT + decodeURIComponent(src.slice(base.length)) : src;
  };
  const NoteImage = Image.extend({
    addAttributes() {
      return {
        ...this.parent?.(),
        // Kept as "attachment:<file>" in the note; shown from wherever the app serves kept files.
        src: { default: null, parseHTML: (el) => unresolve(el.getAttribute("src")), renderHTML: (attrs) => ({ src: resolve(attrs.src) }) },
      };
    },
  });
  return [
    StarterKit.configure({
      heading: { levels: [1, 2, 3] },
      link: { openOnClick: false, autolink: true, protocols: ["attachment"], HTMLAttributes: { rel: "noopener noreferrer" } },
    }),
    NoteImage.configure({ inline: false, allowBase64: false }),
    TaskList,
    TaskItem.configure({ nested: true }),
    TableKit.configure({ table: { resizable: false } }),
    TextAlign.configure({ types: ["heading", "paragraph"] }),
    Markdown,
  ];
}

/// One editor, never shown, that turns Markdown into the same HTML a note shows, for text that is only read.
let reader = null;
let readerFiles = () => "";

/// Markdown as HTML, made from what a note can hold and nothing else, so no markup in the text comes
/// through as it is. `files(file)` gives the address a kept file is shown from.
export function toHTML(markdown, files = () => "") {
  readerFiles = files;
  reader ??= new Editor({ element: document.createElement("div"), editable: false, extensions: noteExtensions((f) => readerFiles(f)) });
  reader.commands.setContent(markdown ?? "", { contentType: "markdown", emitUpdate: false });
  return reader.getHTML();
}

export function create({ element, host, placeholder = "" }) {

  const Shortcuts = Extension.create({
    name: "noteShortcuts",
    addKeyboardShortcuts() {
      return {
        "Mod-k": () => { editLink(); return true; },
        "Mod-Shift-v": () => { markdownPaste = Date.now(); return false; },
      };
    },
    addProseMirrorPlugins() {
      return [dictationPlugin()];
    },
  });

  let markdownPaste = 0;
  // What redraws each toolbar button; filled in below, and run on every change, the first while the editor is made.
  const states = [];
  const content = h("div", { class: "note-body-holder" });
  const toolbar = h("div", { class: "note-toolbar", role: "toolbar", "aria-label": "Formatting" });
  element.append(content);

  const editor = new Editor({
    element: content,
    extensions: [...noteExtensions(host.resolve), Placeholder.configure({ placeholder }), Shortcuts],
    editorProps: {
      attributes: { class: "note-body", id: "note-body", role: "textbox", "aria-multiline": "true", "aria-label": "Note" },
      handlePaste(view, event) {
        const files = [...(event.clipboardData?.files ?? [])];
        if (files.length) { event.preventDefault(); addFiles(files); return true; }
        if (Date.now() - markdownPaste < 1500) {
          markdownPaste = 0;
          const text = event.clipboardData?.getData("text/plain");
          if (text) { event.preventDefault(); insertMarkdown(text); return true; }
        }
        return false;
      },
      handleDrop(view, event) {
        const files = [...(event.dataTransfer?.files ?? [])];
        if (!files.length) return false;
        event.preventDefault();
        const at = view.posAtCoords({ left: event.clientX, top: event.clientY });
        if (at) editor.commands.setTextSelection(at.pos);
        addFiles(files);
        return true;
      },
      handleClick(view, pos, event) {
        // A plain click puts the cursor in a link; Ctrl (or ⌘) + click opens it. Kept files open on a click.
        const link = event.target.closest?.("a[href]");
        if (!link) return false;
        const href = link.getAttribute("href");
        if (href.startsWith(ATTACHMENT) || event.ctrlKey || event.metaKey) { event.preventDefault(); host.open(href); return true; }
        return false;
      },
    },
    onUpdate() { host.onChange(); adoptWebImages(); },
    onTransaction() { drawState(); },
  });

  // ----- Files and pictures

  async function addFiles(files) {
    for (const file of files) {
      try {
        const kept = await host.addFile(file);
        // After whatever is selected, never in place of it: a picture just added stays selected.
        const at = editor.state.selection.to;
        if (kept.image) editor.chain().focus().insertContentAt(at, { type: "image", attrs: { src: ATTACHMENT + kept.file, alt: file.name || kept.name } }).run();
        else editor.chain().focus().insertContentAt(at, [{ type: "text", text: file.name || kept.name, marks: [{ type: "link", attrs: { href: ATTACHMENT + kept.file } }] }, { type: "text", text: " " }]).run();
      } catch (e) {
        host.notify(String(e));
      }
    }
  }

  function pickFiles(accept) {
    const input = h("input", { type: "file", multiple: true, accept, style: "display:none" });
    input.addEventListener("change", () => { addFiles([...input.files]); input.remove(); });
    document.body.append(input);
    input.click();
  }

  /// Pictures on the web are fetched once and kept with the note, so it reads the same offline and
  /// opening it tells no web site anything.
  const adopting = new Set();
  function adoptWebImages() {
    editor.state.doc.descendants((node) => {
      const src = node.type.name === "image" ? node.attrs.src : null;
      if (!src || !/^https?:\/\//i.test(src) || adopting.has(src)) return;
      adopting.add(src);
      host.importImage(src).then((kept) => swapImage(src, ATTACHMENT + kept.file), () => swapImage(src, null));
    });
  }
  function swapImage(from, to) {
    if (editor.isDestroyed) return;
    const tr = editor.state.tr;
    editor.state.doc.descendants((node, pos) => {
      if (node.type.name !== "image" || node.attrs.src !== from) return;
      if (to) tr.setNodeMarkup(pos, undefined, { ...node.attrs, src: to });
      // A picture that couldn't be fetched stays as a link to it.
      else tr.replaceWith(tr.mapping.map(pos), tr.mapping.map(pos + node.nodeSize), editor.schema.nodes.paragraph.create(null, editor.schema.text(from, [editor.schema.marks.link.create({ href: from })])));
    });
    if (tr.docChanged) editor.view.dispatch(tr);
    adopting.delete(from);
  }

  // ----- Small dialogs

  function ask({ title, label, value = "", placeholder: hint = "", multiline = false, action = "Add", remove = false }) {
    return new Promise((done) => {
      const field = multiline ? h("textarea", { rows: "10", placeholder: hint }) : h("input", { type: "text", placeholder: hint, value });
      const dialog = h("dialog", { class: "sheet note-ask", "aria-label": title },
        h("h2", {}, title),
        h("label", { class: "field" }, label, field),
        h("div", { class: "actions" },
          remove ? h("button", { class: "link danger", type: "button", onclick: () => { result = ""; dialog.close(); } }, "Remove") : null,
          h("span", { class: "grow" }),
          h("button", { class: "pill", type: "button", onclick: () => dialog.close() }, "Cancel"),
          h("button", { class: "primary pill", type: "button", onclick: () => { result = field.value; dialog.close(); } }, action)));
      let result = null;
      field.addEventListener("keydown", (e) => { if (e.key === "Enter" && (!multiline || e.ctrlKey || e.metaKey)) { e.preventDefault(); result = field.value; dialog.close(); } });
      dialog.addEventListener("close", () => { dialog.remove(); done(result); });
      document.body.append(dialog);
      dialog.showModal();
      field.focus();
      field.select?.();
    });
  }

  async function editLink() {
    const had = editor.getAttributes("link").href ?? "";
    const href = await ask({ title: had ? "Edit link" : "Add link", label: "Web address", value: had, placeholder: "https://", action: had ? "Save" : "Add", remove: !!had });
    if (href == null) return editor.commands.focus();
    if (!href.trim()) return editor.chain().focus().extendMarkRange("link").unsetLink().run();
    const address = /^[a-z][a-z0-9+.-]*:/i.test(href.trim()) ? href.trim() : `https://${href.trim()}`;
    if (editor.state.selection.empty && !had) editor.chain().focus().insertContent({ type: "text", text: address, marks: [{ type: "link", attrs: { href: address } }] }).run();
    else editor.chain().focus().extendMarkRange("link").setLink({ href: address }).run();
  }

  async function webImage() {
    const url = await ask({ title: "Picture from the web", label: "The picture's address. It's fetched once and kept with the note.", placeholder: "https://" });
    if (!url?.trim()) return editor.commands.focus();
    try {
      const kept = await host.importImage(url.trim());
      editor.chain().focus().insertContentAt(editor.state.selection.to, { type: "image", attrs: { src: ATTACHMENT + kept.file } }).run();
    } catch (e) {
      host.notify(String(e));
    }
  }

  function insertMarkdown(text, at) {
    const parsed = editor.markdown.parse(text);
    if (at != null) editor.chain().insertContentAt(at, parsed.content ?? []).run();
    else editor.chain().focus().insertContent(parsed.content ?? []).run();
  }

  async function pasteMarkdown() {
    let text = null;
    try { text = await navigator.clipboard.readText(); } catch { /* Not allowed to read it: ask for it instead. */ }
    if (!text) text = await ask({ title: "Paste as Markdown", label: "Paste Markdown here. Headings, lists, links and tables become formatting.", multiline: true, action: "Insert" });
    if (text?.trim()) insertMarkdown(text);
    else editor.commands.focus();
  }

  const SHORTCUTS = [
    ["Bold", "Mod+B"], ["Italic", "Mod+I"], ["Underline", "Mod+U"], ["Strikethrough", "Mod+Shift+S"], ["Add link", "Mod+K"],
    ["Heading 1, 2, 3", "Mod+Alt+1 … 3"], ["Bulleted list", "Mod+Shift+8"], ["Numbered list", "Mod+Shift+7"], ["To-do list", "Mod+Shift+9"],
    ["Quote", "Mod+Shift+B"], ["Code block", "Mod+Alt+C"], ["Paste as Markdown", "Mod+Shift+V"], ["Undo", "Mod+Z"], ["Redo", "Mod+Shift+Z"],
    ["Align left, centre, right", "Mod+Shift+L, E, R"], ["Open a link", `${MOD} + click`],
  ];
  const TYPED = [["# then a space", "Heading"], ["- then a space", "Bulleted list"], ["1. then a space", "Numbered list"], ["[ ] then a space", "To-do"], ["> then a space", "Quote"], ["```", "Code block"], ["---", "Divider"], ["**bold**, *italic*, `code`", "As you type"]];
  function showShortcuts() {
    const dialog = h("dialog", { class: "sheet note-shortcuts", "aria-label": "Shortcuts" },
      h("h2", {}, "Shortcuts"),
      h("div", { class: "shortcut-grid" }, SHORTCUTS.map(([what, how]) => [h("span", {}, what), h("kbd", {}, keys(how))])),
      h("h3", {}, "Or just type"),
      h("div", { class: "shortcut-grid" }, TYPED.map(([how, what]) => [h("span", {}, what), h("kbd", {}, how)])),
      h("div", { class: "actions" }, h("button", { class: "pill", type: "button", onclick: () => dialog.close() }, "Close")));
    dialog.addEventListener("close", () => dialog.remove());
    document.body.append(dialog);
    dialog.showModal();
  }

  // ----- The toolbar

  const run = (f) => () => f(editor.chain().focus()).run();
  function button(name, label, shortcut, action, active, enabled) {
    const b = h("button", { class: "ne-btn", type: "button", title: shortcut ? `${label} (${keys(shortcut)})` : label, "aria-label": label }, icon(name));
    b.addEventListener("mousedown", (e) => e.preventDefault()); // The cursor stays in the note.
    b.addEventListener("click", action);
    states.push(() => {
      if (active) { const on = active(); b.classList.toggle("on", on); b.setAttribute("aria-pressed", String(on)); }
      if (enabled) b.disabled = !enabled();
    });
    return b;
  }
  function menu(name, label, items) {
    const wrap = h("span", { class: "ne-menu" });
    const list = h("div", { class: "menu", role: "menu", hidden: true });
    const opener = h("button", { class: "ne-btn", type: "button", title: label, "aria-label": label, "aria-haspopup": "menu", "aria-expanded": "false" }, icon(name));
    const close = () => { list.hidden = true; opener.classList.remove("open"); opener.setAttribute("aria-expanded", "false"); document.removeEventListener("mousedown", outside, true); };
    const outside = (e) => { if (!wrap.contains(e.target)) close(); };
    opener.addEventListener("mousedown", (e) => e.preventDefault());
    opener.addEventListener("click", () => {
      if (!list.hidden) return close();
      // Kept on screen: opens leftwards when the button is near the right edge.
      list.classList.toggle("left", opener.getBoundingClientRect().left + 300 > window.innerWidth);
      list.hidden = false;
      opener.classList.add("open");
      opener.setAttribute("aria-expanded", "true");
      document.addEventListener("mousedown", outside, true);
    });
    list.addEventListener("keydown", (e) => { if (e.key === "Escape") { close(); editor.commands.focus(); } });
    for (const item of items) {
      if (item === "-") { list.append(h("hr")); continue; }
      const [itemIcon, text, shortcut, action, active] = item;
      const row = h("button", { type: "button", role: "menuitem" }, icon(itemIcon, 18), h("span", { class: "grow" }, text), shortcut ? h("kbd", {}, keys(shortcut)) : null);
      row.addEventListener("mousedown", (e) => e.preventDefault());
      row.addEventListener("click", () => { close(); action(); });
      if (active) states.push(() => row.classList.toggle("on", active()));
      list.append(row);
    }
    wrap.append(opener, list);
    return wrap;
  }
  const on = (name, attrs) => () => editor.isActive(name, attrs);
  const sep = () => h("span", { class: "ne-sep" });
  toolbar.append(
    button("undo", "Undo", "Mod+Z", run((c) => c.undo()), null, () => editor.can().undo()),
    button("redo", "Redo", "Mod+Shift+Z", run((c) => c.redo()), null, () => editor.can().redo()),
    sep(),
    button("bold", "Bold", "Mod+B", run((c) => c.toggleBold()), on("bold")),
    button("italic", "Italic", "Mod+I", run((c) => c.toggleItalic()), on("italic")),
    button("h1", "Heading 1", "Mod+Alt+1", run((c) => c.toggleHeading({ level: 1 })), on("heading", { level: 1 })),
    button("h2", "Heading 2", "Mod+Alt+2", run((c) => c.toggleHeading({ level: 2 })), on("heading", { level: 2 })),
    button("bullets", "Bulleted list", "Mod+Shift+8", run((c) => c.toggleBulletList()), on("bulletList")),
    button("numbers", "Numbered list", "Mod+Shift+7", run((c) => c.toggleOrderedList()), on("orderedList")),
    sep(),
    menu("plus", "Insert", [
      ["image", "Picture…", null, () => pickFiles("image/*")],
      ["globe", "Picture from the web…", null, webImage],
      ["file", "File…", null, () => pickFiles("")],
      ["link", "Link…", "Mod+K", editLink],
      "-",
      ["table", "Table", null, run((c) => c.insertTable({ rows: 3, cols: 3, withHeaderRow: true }))],
      ["quote", "Quote", "Mod+Shift+B", run((c) => c.toggleBlockquote()), on("blockquote")],
      ["rule", "Divider", null, run((c) => c.setHorizontalRule())],
    ]),
    menu("more", "More", [
      ["link", "Add link", "Mod+K", editLink, on("link")],
      ["strike", "Strikethrough", "Mod+Shift+S", run((c) => c.toggleStrike()), on("strike")],
      ["todo", "To-do list", "Mod+Shift+9", run((c) => c.toggleTaskList()), on("taskList")],
      ["code", "Code block", "Mod+Alt+C", run((c) => c.toggleCodeBlock()), on("codeBlock")],
      ["paste", "Paste as Markdown", "Mod+Shift+V", pasteMarkdown],
      ["copy", "Copy as Markdown", null, async () => { await navigator.clipboard.writeText(editor.getMarkdown()).catch(() => {}); host.notify("Copied as Markdown."); }],
      "-",
      ["keyboard", "List of shortcuts", null, showShortcuts],
    ]),
    menu("align", "Align", [
      ["align-left", "Align left", "Mod+Shift+L", run((c) => c.setTextAlign("left")), () => editor.isActive({ textAlign: "left" })],
      ["align-center", "Align centre", "Mod+Shift+E", run((c) => c.setTextAlign("center")), () => editor.isActive({ textAlign: "center" })],
      ["align-right", "Align right", "Mod+Shift+R", run((c) => c.setTextAlign("right")), () => editor.isActive({ textAlign: "right" })],
    ]));
  function drawState() { for (const draw of states) draw(); }
  drawState();

  // ----- What the page uses

  const setFrom = ({ doc, markdown, plain }) => {
    if (doc) editor.commands.setContent(doc, { emitUpdate: false });
    else if (markdown != null) editor.commands.setContent(markdown, { contentType: "markdown", emitUpdate: false });
    else editor.commands.setContent(plainToDoc(plain), { emitUpdate: false });
    adoptWebImages();
  };

  return {
    editor,
    toolbar,
    dom: editor.view.dom,
    setFrom,
    json: () => editor.getJSON(),
    markdown: () => editor.getMarkdown(),
    text: () => editor.getText({ blockSeparator: "\n" }),
    isEmpty: () => editor.isEmpty && !dictationKey.getState(editor.state).slots.some((s) => !s.anchor),
    focusEnd: () => editor.commands.focus("end"),
    /// Adds Markdown at the end of the note.
    append(markdown) {
      insertMarkdown(markdown, editor.state.doc.content.size);
      editor.commands.focus("end");
    },
    /// Selects the first place `text` appears, and scrolls to it.
    select(text) {
      const want = text.replace(/^[#>\-*\s\d.[\]x]+/i, "").replace(/[*_`]/g, "").slice(0, 40);
      if (!want) return;
      let found = null;
      editor.state.doc.descendants((node, pos) => {
        if (found || !node.isTextblock) return !found;
        const at = node.textContent.indexOf(want);
        if (at >= 0) found = { from: pos + 1 + at, to: pos + 1 + node.content.size };
        return false;
      });
      if (found) editor.chain().focus().setTextSelection(found).scrollIntoView().run();
    },
    dictation: dictationApi(editor),
    destroy: () => editor.destroy(),
  };
}
