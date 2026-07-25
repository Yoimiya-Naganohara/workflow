import { createHighlighterCoreSync, createJavaScriptRegexEngine } from 'shiki';
import type { HighlighterCore } from 'shiki/core';

let highlighter: HighlighterCore | null = null;
let loadedLangs: string[] = [];
let activeTheme = 'github-light-default';

const engine = createJavaScriptRegexEngine();

export function initHighlighter(
	langs: any[],
	themes: any[],
	defaultTheme: string,
): HighlighterCore {
	if (highlighter) return highlighter;

	highlighter = createHighlighterCoreSync({
		langs,
		themes,
		engine,
	});
	highlighter.setTheme(defaultTheme);
	activeTheme = defaultTheme;
	loadedLangs = highlighter.getLoadedLanguages();
	return highlighter;
}

export function setTheme(theme: string) {
	activeTheme = theme;
	if (highlighter) {
		try {
			highlighter.setTheme(theme);
		} catch {
			// theme not loaded, ignore
		}
	}
}

export function highlightCode(code: string, lang: string): string {
	if (!highlighter) {
		return `<pre class="shiki-fallback"><code>${escapeHtml(code)}</code></pre>`;
	}

	const targetLang = loadedLangs.includes(lang) ? lang : 'text';
	try {
		return highlighter.codeToHtml(code, { lang: targetLang, theme: activeTheme });
	} catch {
		return `<pre class="shiki-fallback"><code>${escapeHtml(code)}</code></pre>`;
	}
}

export function isReady(): boolean {
	return highlighter !== null;
}

function escapeHtml(text: string): string {
	return text
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;')
		.replace(/'/g, '&#039;');
}
