import { EditorState, Compartment } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
} from "@codemirror/view";
import { history, defaultKeymap, historyKeymap } from "@codemirror/commands";
import { search, searchKeymap } from "@codemirror/search";
import { syntaxHighlighting, defaultHighlightStyle, StreamLanguage } from "@codemirror/language";
import { oneDark } from "@codemirror/theme-one-dark";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { html } from "@codemirror/lang-html";
import { css } from "@codemirror/lang-css";
import { python } from "@codemirror/lang-python";
import { markdown } from "@codemirror/lang-markdown";
import { rust } from "@codemirror/lang-rust";
import { toml } from "@codemirror/legacy-modes/mode/toml";

const params = new URLSearchParams(location.search);
const token = params.get("token") || "";

const tabsEl = document.getElementById("tabs");
const noticeEl = document.getElementById("notice");
const wrapButton = document.getElementById("wrap");
const editorParent = document.getElementById("editor");

const wrapComp = new Compartment();
const langComp = new Compartment();
const themeComp = new Compartment();

let since = null;
let interval = 500;
let idle204 = 0;
let pollTimer = 0;
let activeId = null;
let activeName = "";
let loadGen = 0;
let reported = false;
let dark = false;
let wrapping = false;

function languageOf(name) {
  const ext = name.includes(".") ? name.split(".").pop().toLowerCase() : "";
  switch (ext) {
    case "js":
    case "mjs":
    case "cjs":
      return javascript();
    case "json":
      return json();
    case "html":
    case "htm":
      return html();
    case "css":
      return css();
    case "py":
      return python();
    case "md":
    case "markdown":
      return markdown();
    case "rs":
      return rust();
    case "toml":
      return StreamLanguage.define(toml);
    default:
      return [];
  }
}

function extensions(name) {
  return [
    lineNumbers(),
    highlightActiveLine(),
    highlightActiveLineGutter(),
    drawSelection(),
    history(),
    EditorView.editable.of(false),
    EditorState.readOnly.of(true),
    syntaxHighlighting(defaultHighlightStyle),
    search({ top: true }),
    keymap.of([...searchKeymap, ...defaultKeymap, ...historyKeymap]),
    wrapComp.of(wrapping ? EditorView.lineWrapping : []),
    langComp.of(languageOf(name)),
    themeComp.of(dark ? oneDark : []),
    EditorView.theme({
      "&": { height: "100%" },
      ".cm-scroller": { overflow: "auto" },
    }),
  ];
}

const view = new EditorView({
  parent: editorParent,
  state: EditorState.create({ doc: "", extensions: extensions("") }),
});

wrapButton.addEventListener("click", () => {
  wrapping = !wrapping;
  wrapButton.classList.toggle("on", wrapping);
  wrapButton.setAttribute("aria-pressed", wrapping ? "true" : "false");
  view.dispatch({
    effects: wrapComp.reconfigure(wrapping ? EditorView.lineWrapping : []),
  });
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !event.defaultPrevented) {
    postEvent({ type: "close" });
  }
});

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") schedule(0);
});

window.addEventListener("error", (event) => {
  if (reported) return;
  reported = true;
  postEvent({ type: "error", message: String(event.message || "error") });
});

function showNotice(text) {
  if (!text) {
    noticeEl.style.display = "none";
    noticeEl.textContent = "";
    return;
  }
  noticeEl.style.display = "block";
  noticeEl.textContent = text;
}

function replaceDoc(name, text) {
  view.setState(EditorState.create({ doc: text, extensions: extensions(name) }));
}

function applyTheme(next) {
  dark = next;
  document.body.classList.toggle("dark", dark);
  view.dispatch({ effects: themeComp.reconfigure(dark ? oneDark : []) });
}

async function postEvent(body) {
  if (!token) return;
  await fetch("/event?token=" + encodeURIComponent(token), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

async function loadFile(id, name, gen) {
  const res = await fetch(
    "/file?token=" + encodeURIComponent(token) + "&id=" + encodeURIComponent(id),
  );
  if (gen !== loadGen || id !== activeId) return;
  if (res.status === 503) {
    setTimeout(() => loadFile(id, name, gen), 1000);
    return;
  }
  const text = await res.text();
  if (gen !== loadGen || id !== activeId) return;
  if (!res.ok) {
    showNotice(text || "Cannot read this file");
    replaceDoc(name, "");
    return;
  }
  showNotice(res.headers.get("X-Preview-Truncated") === "1" ? "Truncated at 5 MiB" : "");
  replaceDoc(name, text);
}

function renderSession(data) {
  const docs = Array.isArray(data["open-docs"]) ? data["open-docs"] : [];
  const counts = {};
  docs.forEach((doc) => {
    counts[doc.name] = (counts[doc.name] || 0) + 1;
  });
  tabsEl.textContent = "";
  docs.forEach((doc) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "tab" + (doc.id === data.active ? " active" : "");
    const label = doc.name || "file";
    button.title =
      counts[doc.name] > 1 ? label + " (" + String(doc.id).slice(0, 8) + ")" : label;
    button.addEventListener("click", () => postEvent({ type: "select-tab", id: doc.id }));
    const name = document.createElement("span");
    name.className = "tab-name";
    name.textContent = label;
    const close = document.createElement("span");
    close.className = "tab-close";
    close.textContent = "\u00d7";
    close.title = "Close";
    close.addEventListener("click", (event) => {
      event.stopPropagation();
      postEvent({ type: "close-tab", id: doc.id });
    });
    button.append(name, close);
    tabsEl.appendChild(button);
  });
  if (data.theme && typeof data.theme.dark === "boolean") applyTheme(data.theme.dark);
  const next = data.active || null;
  if (next !== activeId) {
    activeId = next;
    const doc = docs.find((item) => item.id === next);
    activeName = doc ? doc.name || "" : "";
    loadGen += 1;
    if (next) loadFile(next, activeName, loadGen);
    else replaceDoc("", "");
  }
}

async function poll() {
  if (!token) {
    showNotice("Missing token");
    return;
  }
  let res;
  try {
    const query = since === null ? "" : "&since=" + encodeURIComponent(String(since));
    res = await fetch("/session?token=" + encodeURIComponent(token) + query);
  } catch (_error) {
    schedule(interval);
    return;
  }
  if (res.status === 204) {
    idle204 += 1;
    if (idle204 >= 2) interval = 2000;
  } else if (res.ok) {
    idle204 = 0;
    interval = 500;
    const data = await res.json();
    since = data.version;
    renderSession(data);
  }
  schedule(interval);
}

function schedule(delay) {
  clearTimeout(pollTimer);
  pollTimer = setTimeout(poll, delay);
}

// The macOS panel calls this when the session changes. It only starts a pull.
window.__ebPreviewWake = function () {
  schedule(0);
};

postEvent({ type: "ready" }).then(() => schedule(0));
