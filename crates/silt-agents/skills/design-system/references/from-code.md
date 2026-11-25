# A design system from a project's code

Read this when the system is built from a repository: the project you're in, another checkout on this machine, or a repository the person names. Its custom properties or token files become `tokens.json`, its font files, marks and docs the fonts, assets and README, and its components the examples and guidelines.

**The repository is read-only for this.** Never push, branch or edit its code; the only files you write are the system's. Read only regular files whose real path is inside the checkout: skip symbolic links and references (`@font-face` URLs, imports) that point outside it, and say so. Run its install or build only with the person's go-ahead, on code they vouch for.

## First sync

1. **Ask once for what's missing.** Which repository, branch and package (in a monorepo, the one that owns the styles and components); what to bring in (colour and type tokens, spacing and radii, font files, logos and icons, docs, components); and, if no name was given, the system's name. The ref is the branch's head unless told otherwise; note its short commit.
2. **Orient cheaply.** Find the package that owns the styles and components (its `package.json`), and list by path only what you'll read:
   - a token file (`tokens.json`, DTCG `*.tokens.json`, a theme object, `tailwind.config.*`), or the CSS files declaring `--name: value` under `:root`, theme selectors or a Tailwind 4 `@theme { … }` block;
   - `@font-face` rules and `*.woff2`, `*.woff`, `*.ttf`, `*.otf` files;
   - SVG or PNG marks under logo, brand or icon folders;
   - the components: the entry's exports, stories (`*.stories.*`), component docs (`*.md`, `*.mdx`);
   - the README and any brand or design docs.

   Never read built `dist/` bundles, lockfiles or whole directories. Show the inventory before writing: N colour variables by M themes, K text styles, the fonts, the marks, and the components by name.
3. **Tokens, exactly as the source has them.**
   - Each theme selector becomes one `color.themes` entry: `:root` or light first; `[data-theme="dark"]`, `.dark` or a `prefers-color-scheme: dark` block becomes `dark`.
   - `--name` becomes the name as written, without `--`; `/` and `.` become `-`.
   - The value as written: hex lowercased, `rgb()`, `hsl()` and `oklch()` kept. A colour that's `var(--other)` becomes the alias `"{other}"`. A length or shadow that refers to another variable gets that variable's literal value. `calc()`, `color-mix()` or a JavaScript expression is skipped and noted.
   - The family by what it is: a colour goes in `color`; lengths by name (`space`, `gap`, `pad` in `spacing`; `radius`, `rounded` in `radius`; `shadow` in `shadow`); font sizes, line heights, weights and families become `type` styles and `type.families`. Pair a size with its line height; a Tailwind or theme-object scale maps one to one.
   - `usage` is the comment on that line or the doc's sentence, in your words, else where the code uses it.
   - A `tokens.json` already in this format is copied. Never invent a value; list what you couldn't place.
4. **Fonts and assets are copied, never approximated.** Each font file the CSS names goes in `fonts/` with a `type.fonts` entry (family, file, weight and style from its `@font-face`), each at most 1 MiB. Logo and icon files go verbatim into `assets/Logos/` and `assets/Icons/`.
5. **Docs.** The README is the repository's brand and usage guidance turned into usage rules that name tokens, never install instructions. A component's doc becomes its `components/<name>.md`.
6. **Components: list them, ask, then write them.** List the exported components (name, stories, doc) and ask which to include. Contexts, providers and hooks aren't components here. For each one chosen:
   - copy its rules into `components.css` from its stylesheet, or from the classes its markup uses (a Tailwind component's utilities resolved to their values over the system's tokens);
   - write its example from its default story and a few telling variants, as plain HTML with those classes;
   - write its guidelines from its doc and its props.

   When its look lives in code you can't read as markup, write it from the rendered result and say so in its guidelines.
7. **Record the source** in `tokens.json`: `"meta": {"source": "code", "repo": "owner/name", "ref": "main@1a2b3c4", "package": "packages/ui", "paths": {"tokens": [...], "fonts": [...], "assets": [...], "docs": [...]}, "components": {"button": "src/Button.tsx"}, "synced": "<date>"}`. The README gets one "Not synced" note: variables skipped, fonts not fetched, components not written.
8. **Finish** with the cover, then tell the person in two sentences what came from which commit and what didn't.

## Syncing again

The same request on a system whose `meta.source` is `code` syncs from what the person names now, never from what `meta` stored: tell them what it recorded last time if it differs. Run steps 2 to 6 again, then merge as the skill's "Revising and syncing again" says: update changed values and components, add what's new, keep notes, prose, fonts, assets and anything a person added, and list the tokens the code no longer defines, asking before removing them. Update `meta`.
