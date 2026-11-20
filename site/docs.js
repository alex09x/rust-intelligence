/*
 * rust-intelligence — renders docs/ARCHITECTURE.md (copied to architecture.md at deploy) on docs.html
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */
"use strict";

// Relative links in the document are relative to docs/ in the repository.
const REPO_DOCS = "https://github.com/alex09x/rust-intelligence/blob/main/docs/";
const SITE_ASSETS = "../site/assets/";

const slug = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");

function fixLinks(root) {
  root.querySelectorAll("img[src]").forEach((img) => {
    const src = img.getAttribute("src");
    if (src.startsWith(SITE_ASSETS)) img.setAttribute("src", "assets/" + src.slice(SITE_ASSETS.length));
    img.setAttribute("loading", "lazy");
  });
  root.querySelectorAll("a[href]").forEach((a) => {
    const href = a.getAttribute("href");
    if (/^(https?:|mailto:|#)/.test(href)) return;
    a.setAttribute("href", new URL(href, REPO_DOCS).href);
  });
}

function buildToc(root) {
  const toc = document.getElementById("toc");
  root.querySelectorAll("h2, h3").forEach((h) => {
    h.id = slug(h.textContent);
    const a = document.createElement("a");
    a.href = "#" + h.id;
    a.textContent = h.textContent;
    if (h.tagName === "H3") a.className = "sub";
    toc.appendChild(a);
  });
}

async function main() {
  const article = document.getElementById("top");
  try {
    const res = await fetch("architecture.md", { cache: "no-cache" });
    if (!res.ok) throw new Error(String(res.status));
    const md = await res.text();
    article.innerHTML = window.marked.parse(md, { gfm: true });
    article.querySelectorAll("table").forEach((t) => {
      const wrap = document.createElement("div");
      wrap.className = "tbl";
      t.replaceWith(wrap);
      wrap.appendChild(t);
    });
    fixLinks(article);
    buildToc(article);
    if (location.hash) document.getElementById(location.hash.slice(1))?.scrollIntoView();
  } catch (e) {
    const status = document.getElementById("doc-status");
    if (status) status.firstChild.textContent = "The document could not be loaded here. Read it on ";
  }
}

main();
