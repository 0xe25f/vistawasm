import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";

// Builds the GitHub Pages site into _site/:
//
// - site/*.html pages, with the shared navigation and footer from
//   site/_nav.html and site/_footer.html;
// - the three.js and Babylon.js guides, rendered from docs/threejs.md and
//   docs/babylonjs.md, so the site never drifts from the tested guides;
// - the demo, at demo/, with the compiled package that
//   `npm run build:demo` copied into demo/dist;
// - the screenshots in docs/images.
//
// It then checks every link and anchor: each local file must exist, each
// #fragment must name an id on its page, and each link into this
// repository on GitHub must name a file that exists here.

const OUT = "_site";
const REPOSITORY = "https://github.com/0xe25f/vistawasm";
const GUIDES = [
  {
    name: "threejs",
    title: "Using VistaWASM with three.js",
    lead: "Draw three.js objects over a VistaWASM world with one shared camera, or build VistaWASM terrain into your own three.js scene.",
    example: `${REPOSITORY}/tree/main/examples/threejs`,
    other: "babylonjs"
  },
  {
    name: "babylonjs",
    title: "Using VistaWASM with Babylon.js",
    lead: "Draw Babylon.js meshes over a VistaWASM world, with hills that hide them, or build VistaWASM terrain into your own Babylon.js scene.",
    example: `${REPOSITORY}/tree/main/examples/babylonjs`,
    other: "threejs"
  }
];
const GUIDE_NAMES = { threejs: "three.js", babylonjs: "Babylon.js" };

function fail(message) {
  console.error(message);
  process.exit(1);
}

function escapeHtml(text) {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/** GitHub's heading anchors: lower case, punctuation dropped, spaces as hyphens. */
function slug(text) {
  return text.replace(/`/g, "").toLowerCase().replace(/[^\p{L}\p{N} _-]/gu, "").replace(/ /g, "-");
}

/** Where a link in docs/*.md points once the page is on the site. */
function rewriteUrl(url) {
  if (/^(https?:|mailto:|#)/.test(url)) {
    return url;
  }

  const doc = url.match(/^([\w-]+)\.md(#.*)?$/);

  if (doc) {
    return GUIDE_NAMES[doc[1]] ? `${doc[1]}.html${doc[2] ?? ""}` : `${REPOSITORY}/blob/main/docs/${doc[1]}.md${doc[2] ?? ""}`;
  }

  if (url.startsWith("../")) {
    const path = url.slice(3);
    return `${REPOSITORY}/${path.endsWith("/") ? "tree" : "blob"}/main/${path}`;
  }

  return `${REPOSITORY}/blob/main/docs/${url}`;
}

const INLINE = /(`[^`]+`)|\[((?:[^\]`]|`[^`]*`)+)\]\(([^)\s]+)\)|\*\*([^*]+)\*\*|\*([^*\s][^*]*)\*/g;

/** Inline Markdown: code, links, bold and emphasis. Everything else is escaped text. */
function inline(text) {
  let html = "";
  let last = 0;

  for (const match of text.matchAll(INLINE)) {
    html += escapeHtml(text.slice(last, match.index));
    const [, code, label, url, bold, emphasis] = match;

    if (code) {
      html += `<code>${escapeHtml(code.slice(1, -1))}</code>`;
    } else if (label) {
      html += `<a href="${escapeHtml(rewriteUrl(url))}">${inline(label)}</a>`;
    } else if (bold) {
      html += `<strong>${inline(bold)}</strong>`;
    } else {
      html += `<em>${inline(emphasis)}</em>`;
    }

    last = match.index + match[0].length;
  }

  return html + escapeHtml(text.slice(last));
}

function renderList(lines) {
  const indent = lines[0].match(/^\s*/)[0].length;
  const items = [];

  for (const line of lines) {
    const marker = line.match(/^(\s*)- (.*)$/);

    if (marker && marker[1].length === indent) {
      items.push({ text: [marker[2]], children: [] });
    } else if (marker || items.at(-1).children.length > 0) {
      items.at(-1).children.push(line);
    } else {
      items.at(-1).text.push(line.trim());
    }
  }

  const body = items.map((item) => {
    const children = item.children.length > 0 ? renderList(item.children) : "";
    return `<li>${inline(item.text.join(" "))}${children}</li>`;
  });
  return `<ul>${body.join("")}</ul>`;
}

function renderTable(lines) {
  const cells = (line) => line.replace(/\\\|/g, "\u0000").replace(/^\||\|$/g, "").split("|").map((cell) => inline(cell.trim().replace(/\u0000/g, "|")));
  const [head, , ...rows] = lines;
  const header = cells(head).map((cell) => `<th scope="col">${cell}</th>`).join("");
  const body = rows.map((row) => `<tr>${cells(row).map((cell) => `<td>${cell}</td>`).join("")}</tr>`).join("");
  return `<div class="table-wrap"><table><thead><tr>${header}</tr></thead><tbody>${body}</tbody></table></div>`;
}

/** The Markdown the docs use: headings, paragraphs, lists, tables and fenced code. */
function renderMarkdown(markdown) {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");
  const blocks = [];
  const headings = [];
  let title = "";
  let index = 0;
  const isBlockStart = (line) => /^(```|#{1,6} |\||\s*- )/.test(line);

  while (index < lines.length) {
    const line = lines[index];
    const fence = line.match(/^```(\w*)/);
    const heading = line.match(/^(#{1,6}) (.*)$/);

    if (fence) {
      const code = [];
      index += 1;

      while (index < lines.length && !lines[index].startsWith("```")) {
        code.push(lines[index]);
        index += 1;
      }

      index += 1;
      blocks.push(`<div class="code"><pre><code class="language-${fence[1] || "text"}">${escapeHtml(code.join("\n"))}</code></pre></div>`);
    } else if (heading) {
      const level = heading[1].length;
      const text = heading[2];
      index += 1;

      if (level === 1) {
        title = text;
        continue;
      }

      const id = slug(text);
      blocks.push(`<h${level} id="${id}">${inline(text)}</h${level}>`);

      if (level === 2) {
        headings.push({ id, html: inline(text) });
      }
    } else if (line.startsWith("|")) {
      const rows = [];

      while (index < lines.length && lines[index].startsWith("|")) {
        rows.push(lines[index]);
        index += 1;
      }

      blocks.push(renderTable(rows));
    } else if (/^\s*- /.test(line)) {
      const items = [];

      while (index < lines.length && lines[index].trim() !== "" && (/^\s*- /.test(lines[index]) || /^\s+\S/.test(lines[index]))) {
        items.push(lines[index]);
        index += 1;
      }

      blocks.push(renderList(items));
    } else if (line.trim() === "") {
      index += 1;
    } else {
      const text = [];

      while (index < lines.length && lines[index].trim() !== "" && !isBlockStart(lines[index])) {
        text.push(lines[index].trim());
        index += 1;
      }

      blocks.push(`<p>${inline(text.join(" "))}</p>`);
    }
  }

  return { title, headings, html: blocks.map((block) => `          ${block}`).join("\n") };
}

function fill(template, values) {
  return template.replace(/\{\{(\w+)\}\}\n?/g, (whole, key) => {
    if (!(key in values)) {
      fail(`scripts/build-site.mjs has no value for {{${key}}}.`);
    }

    return values[key];
  });
}

/** Mark the navigation link for `page` as the current page. */
function markCurrent(nav, page) {
  return nav.replace(`<a href="${page}">`, `<a href="${page}" aria-current="page">`);
}

function htmlFiles(directory) {
  return readdirSync(directory, { recursive: true })
    .filter((name) => name.endsWith(".html"))
    .map((name) => join(directory, name));
}

function idsOf(html) {
  return new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((match) => match[1]));
}

function checkLinks() {
  const problems = [];
  const pages = new Map(htmlFiles(OUT).map((path) => [path, readFileSync(path, "utf8")]));

  for (const [path, html] of pages) {
    for (const [, url] of html.matchAll(/\s(?:href|src)="([^"]+)"/g)) {
      const link = url.replace(/&amp;/g, "&");
      const where = `${relative(OUT, path)}: ${link}`;

      if (link.startsWith(`${REPOSITORY}/blob/main/`) || link.startsWith(`${REPOSITORY}/tree/main/`)) {
        const file = link.replace(/^.*?\/(?:blob|tree)\/main\//, "").replace(/#.*$/, "");

        if (!existsSync(file)) {
          problems.push(`${where} names ${file}, which is not in the repository`);
        }

        continue;
      }

      if (/^(https?:|mailto:|data:)/.test(link)) {
        continue;
      }

      const [file, fragment] = link.split("#");
      let target = file === "" ? path : join(dirname(path), file);

      if (existsSync(target) && statSync(target).isDirectory()) {
        target = join(target, "index.html");
      }

      if (!existsSync(target)) {
        problems.push(`${where} points to a missing file`);
        continue;
      }

      if (fragment && target.endsWith(".html") && !idsOf(pages.get(target) ?? readFileSync(target, "utf8")).has(fragment)) {
        problems.push(`${where} points to #${fragment}, which is not an id on that page`);
      }
    }
  }

  if (problems.length > 0) {
    fail(`The site has broken links:\n- ${problems.join("\n- ")}`);
  }
}

if (!existsSync("demo/dist/index.js")) {
  fail("demo/dist/ does not exist yet. Run `npm run build:demo` first, or `npm run build:site`, which runs it.");
}

rmSync(OUT, { force: true, recursive: true });
mkdirSync(OUT, { recursive: true });

const nav = readFileSync("site/_nav.html", "utf8");
const footer = readFileSync("site/_footer.html", "utf8");

for (const name of readdirSync("site")) {
  if (name.startsWith("_")) {
    continue;
  }

  if (name.endsWith(".html")) {
    const page = readFileSync(join("site", name), "utf8");
    writeFileSync(join(OUT, name), fill(page, { nav: markCurrent(nav, name), footer }));
  } else {
    cpSync(join("site", name), join(OUT, name), { recursive: true });
  }
}

const template = readFileSync("site/_guide.html", "utf8");

for (const guide of GUIDES) {
  const { headings, html } = renderMarkdown(readFileSync(`docs/${guide.name}.md`, "utf8"));
  const toc = [
    "          <ol>",
    ...headings.map((heading) => `            <li><a href="#${heading.id}">${heading.html}</a></li>`),
    "          </ol>\n"
  ].join("\n");
  const page = fill(template, {
    nav: markCurrent(nav, `${guide.name}.html`),
    footer,
    title: escapeHtml(guide.title),
    description: escapeHtml(guide.lead),
    lead: escapeHtml(guide.lead),
    example: guide.example,
    toc,
    content: `${html}\n`,
    otherHref: `${guide.other}.html`,
    otherTitle: escapeHtml(GUIDES.find((other) => other.name === guide.other).title),
    otherName: GUIDE_NAMES[guide.other]
  });
  writeFileSync(join(OUT, `${guide.name}.html`), page);
}

cpSync("docs/images", join(OUT, "assets", "images"), { recursive: true });
mkdirSync(join(OUT, "demo"));
cpSync("demo/index.html", join(OUT, "demo", "index.html"));
cpSync("demo/src", join(OUT, "demo", "src"), { recursive: true });
cpSync("demo/dist", join(OUT, "demo", "dist"), { recursive: true });

checkLinks();
console.log(`Built the site in ${OUT}/.`);
