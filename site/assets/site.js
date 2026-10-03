// Small enhancements for the VistaWASM site. Every page reads fully
// without them: copy buttons, code colouring, the live star count and the
// table of contents' current section.

const REPOSITORY = "0xe25f/vistawasm";
const STAR_CACHE_KEY = "vistawasm-stars";
const STAR_CACHE_MS = 60 * 60 * 1000;

const KEYWORDS = new Set([
  "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do",
  "else", "export", "extends", "false", "finally", "for", "from", "function", "if", "import", "in", "instanceof",
  "interface", "let", "new", "null", "of", "return", "switch", "this", "throw", "true", "try", "type", "typeof",
  "undefined", "var", "void", "while", "yield"
]);

// Comments, strings, numbers, then words. Each match becomes one token.
const SCRIPT_TOKENS = /(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|("(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'|`(?:[^`\\]|\\.)*`)|(\b\d[\d_]*(?:\.\d+)?\b)|([A-Za-z_$][\w$]*)/g;
const SHELL_TOKENS = /(#[^\n]*)|("(?:[^"\\\n]|\\.)*"|'[^'\n]*')/g;
const HTML_TOKENS = /(<!--[\s\S]*?-->)|(<\/?[a-zA-Z][\w-]*)|([a-zA-Z-]+)(?==)|("[^"]*")/g;

function span(className, text) {
  const element = document.createElement("span");
  element.className = className;
  element.textContent = text;
  return element;
}

function scriptClass(match, text, rest) {
  if (match[1]) {
    return "tok-comment";
  }

  if (match[2]) {
    return "tok-string";
  }

  if (match[3]) {
    return "tok-number";
  }

  if (KEYWORDS.has(text)) {
    return "tok-keyword";
  }

  if (/^[A-Z]/.test(text)) {
    return "tok-type";
  }

  return /^\s*\(/.test(rest) ? "tok-call" : null;
}

function shellClass(match) {
  return match[1] ? "tok-comment" : "tok-string";
}

function htmlClass(match) {
  if (match[1]) {
    return "tok-comment";
  }

  if (match[2]) {
    return "tok-tag";
  }

  return match[3] ? "tok-attr" : "tok-string";
}

/** Colour a code block by building spans, never by parsing HTML. */
function highlight(code) {
  const language = code.className.match(/language-(\w+)/)?.[1];
  const rules = {
    ts: [SCRIPT_TOKENS, scriptClass],
    js: [SCRIPT_TOKENS, scriptClass],
    bash: [SHELL_TOKENS, shellClass],
    html: [HTML_TOKENS, htmlClass]
  }[language];

  if (!rules) {
    return;
  }

  const [pattern, classify] = rules;
  const source = code.textContent;
  const fragment = document.createDocumentFragment();
  let last = 0;

  for (const match of source.matchAll(pattern)) {
    const text = match[0];
    const className = classify(match, text, source.slice(match.index + text.length, match.index + text.length + 4));

    if (!className) {
      continue;
    }

    fragment.append(source.slice(last, match.index), span(className, text));
    last = match.index + text.length;
  }

  fragment.append(source.slice(last));
  code.replaceChildren(fragment);
}

async function copyText(button, text) {
  const label = button.textContent;

  try {
    await navigator.clipboard.writeText(text);
    button.textContent = "Copied";
  } catch (error) {
    // Clipboard access needs a secure context and permission; say so on the button.
    button.textContent = "Press Ctrl+C";
    console.info("Could not copy to the clipboard:", error);
  }

  setTimeout(() => {
    button.textContent = label;
  }, 1600);
}

function addCopyButtons() {
  for (const block of document.querySelectorAll(".code")) {
    const code = block.querySelector("code");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "copy";
    button.textContent = "Copy";
    button.setAttribute("aria-label", "Copy this code");
    button.addEventListener("click", () => copyText(button, code.textContent));
    block.append(button);
  }

  for (const button of document.querySelectorAll("[data-copy]")) {
    button.addEventListener("click", () => copyText(button, button.dataset.copy));
  }
}

function readCachedStars() {
  try {
    const cached = JSON.parse(sessionStorage.getItem(STAR_CACHE_KEY) ?? "null");
    return cached && Date.now() - cached.at < STAR_CACHE_MS ? cached.count : null;
  } catch {
    // Storage can be blocked; fetch the count instead.
    return null;
  }
}

async function showStars() {
  const targets = document.querySelectorAll("[data-stars]");

  if (targets.length === 0) {
    return;
  }

  let count = readCachedStars();

  if (count === null) {
    try {
      const response = await fetch(`https://api.github.com/repos/${REPOSITORY}`, {
        headers: { Accept: "application/vnd.github+json" }
      });

      if (!response.ok) {
        throw new Error(`GitHub answered ${response.status}`);
      }

      count = (await response.json()).stargazers_count;

      try {
        sessionStorage.setItem(STAR_CACHE_KEY, JSON.stringify({ count, at: Date.now() }));
      } catch {
        // Storage can be blocked; the count still shows.
      }
    } catch (error) {
      // The buttons read well without a count, so leave them as they are.
      console.info("Could not read the star count from GitHub:", error);
      return;
    }
  }

  if (typeof count !== "number") {
    return;
  }

  const text = count >= 1000 ? `${(count / 1000).toFixed(1).replace(/\.0$/, "")}k` : String(count);

  for (const target of targets) {
    target.textContent = text;
    target.setAttribute("aria-label", `${count} stars`);
    target.hidden = false;
  }
}

/** Mark the table of contents entry for the section being read. */
function followContents() {
  const links = [...document.querySelectorAll(".toc a[href^='#']")];
  const sections = links.map((link) => document.getElementById(link.hash.slice(1))).filter(Boolean);

  if (sections.length === 0) {
    return;
  }

  const observer = new IntersectionObserver((entries) => {
    const visible = entries.filter((entry) => entry.isIntersecting);

    if (visible.length === 0) {
      return;
    }

    const id = visible.sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0].target.id;

    for (const link of links) {
      link.setAttribute("aria-current", String(link.hash === `#${id}`));
    }
  }, { rootMargin: "-80px 0px -65% 0px" });

  for (const section of sections) {
    observer.observe(section);
  }
}

for (const code of document.querySelectorAll("pre code")) {
  highlight(code);
}

addCopyButtons();
followContents();
showStars();
