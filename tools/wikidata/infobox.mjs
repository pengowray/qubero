// Reads the file extensions out of an English Wikipedia article's
// {{Infobox file format}}, for the formats whose Wikidata item has an article.
//
// The infobox is written by hand, so the extensions come in every shape
// wikitext allows: `{{code|.gif}}`, a bulleted `{{Plain list}}`, `<code>`
// tags, commas, footnotes. Only words that start with a dot are taken, which
// skips the prose ("none", "varies") and the footnotes' contents alike.

const INFOBOX = /\{\{\s*infobox[ _]file[ _]format\s*[|}]/gi;
const PARAMS = new Set(["extension", "extensions"]);

/** The text of every {{Infobox file format}} in the article, braces balanced. */
function infoboxes(text) {
  const out = [];
  for (const m of text.matchAll(INFOBOX)) {
    let depth = 0;
    for (let i = m.index; i < text.length - 1; i++) {
      if (text[i] === "{" && text[i + 1] === "{") {
        depth++;
        i++;
      } else if (text[i] === "}" && text[i + 1] === "}") {
        depth--;
        i++;
        if (depth === 0) {
          out.push(text.slice(m.index + 2, i - 1));
          break;
        }
      }
    }
  }
  return out;
}

/** The template's parameters, split on the pipes that belong to it and not
 *  to a template or link inside a value. */
function params(body) {
  const out = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < body.length; i++) {
    const two = body.slice(i, i + 2);
    if (two === "{{" || two === "[[") {
      depth++;
      i++;
    } else if (two === "}}" || two === "]]") {
      depth--;
      i++;
    } else if (body[i] === "|" && depth === 0) {
      out.push(body.slice(start, i));
      start = i + 1;
    }
  }
  out.push(body.slice(start));
  return out.slice(1);
}

/**
 * The extensions an article's infoboxes list, lowercase and without the dot,
 * or null when the article has no infobox with an extension parameter.
 */
export function infoboxExtensions(text) {
  let any = false;
  const found = new Set();
  for (const box of infoboxes(text)) {
    for (const p of params(box)) {
      const eq = p.indexOf("=");
      if (eq < 0) continue;
      const name = p.slice(0, eq).trim().toLowerCase().replace(/[ _]/g, "");
      if (!PARAMS.has(name)) continue;
      any = true;
      const value = p
        .slice(eq + 1)
        .replace(/<!--[\s\S]*?-->/g, "")
        .replace(/<ref[^>]*\/>/gi, "")
        .replace(/<ref[\s\S]*?<\/ref>/gi, "");
      for (const m of value.matchAll(/(?:^|[\s,;/(>*|'"[])\.([A-Za-z0-9][A-Za-z0-9_+~-]*(?:\.[A-Za-z0-9_+~-]+)*)/g)) {
        found.add(m[1].toLowerCase());
      }
    }
  }
  return any ? [...found].sort() : null;
}
