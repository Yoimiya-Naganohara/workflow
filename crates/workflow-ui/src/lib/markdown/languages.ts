// ── Shiki language imports ──────────────────────────────────────
// All supported languages for syntax highlighting.
import js from "shiki/langs/javascript.mjs";
import ts from "shiki/langs/typescript.mjs";
import py from "shiki/langs/python.mjs";
import rs from "shiki/langs/rust.mjs";
import json from "shiki/langs/json.mjs";
import html from "shiki/langs/html.mjs";
import css from "shiki/langs/css.mjs";
import shellscript from "shiki/langs/shellscript.mjs";
import sql from "shiki/langs/sql.mjs";
import md from "shiki/langs/markdown.mjs";
import yaml from "shiki/langs/yaml.mjs";
import xml from "shiki/langs/xml.mjs";
import toml from "shiki/langs/toml.mjs";
import go from "shiki/langs/go.mjs";
import rb from "shiki/langs/ruby.mjs";
import java from "shiki/langs/java.mjs";
import c from "shiki/langs/c.mjs";
import cpp from "shiki/langs/cpp.mjs";
import php from "shiki/langs/php.mjs";
import diff from "shiki/langs/diff.mjs";
import graphql from "shiki/langs/graphql.mjs";
import ini from "shiki/langs/ini.mjs";
import kt from "shiki/langs/kotlin.mjs";
import lua from "shiki/langs/lua.mjs";
import make from "shiki/langs/make.mjs";
import perl from "shiki/langs/perl.mjs";
import r from "shiki/langs/r.mjs";
import scala from "shiki/langs/scala.mjs";
import swift from "shiki/langs/swift.mjs";
import svelte from "shiki/langs/svelte.mjs";
import docker from "shiki/langs/docker.mjs";
import solidity from "shiki/langs/solidity.mjs";
import zig from "shiki/langs/zig.mjs";

import githubDark from "shiki/themes/github-dark-default.mjs";
import githubLight from "shiki/themes/github-light-default.mjs";

export const LANGUAGES = [
	js, ts, py, rs, json, html, css, shellscript, sql, md,
	yaml, xml, toml, go, rb, java, c, cpp, php, diff, graphql,
	ini, kt, lua, make, perl, r, scala, swift, svelte, docker,
	solidity, zig,
].flat();

export const THEMES = [githubDark, githubLight];

export const DEFAULT_DARK_THEME = "github-dark-default";
export const DEFAULT_LIGHT_THEME = "github-light-default";

let activeTheme = DEFAULT_LIGHT_THEME;

export function getActiveTheme(): string {
	return activeTheme;
}

export function setActiveTheme(theme: string) {
	activeTheme = theme;
}
