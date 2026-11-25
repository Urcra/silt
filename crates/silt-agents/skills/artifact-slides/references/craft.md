# Deck craft: space, type and layouts

The numbers here are for the 16:9 stage, 1280×720. For 4:3 (1024×768), keep the same type sizes and fit the content to the narrower width.

## The space you have

- **Margins:** 64 px on every side leaves 1152×592 for content. Go no tighter than 48 px.
- **A footer band** (a source, a page number, a logo) is one row of 20 px text pinned 32 px above the bottom edge. A slide with one gets `padding-bottom: 96px`, and nothing else goes below y 640.
- **Measure before you place.** Text wraps only at spaces, and nothing scrolls, so overflow is silent:
  - a line of text is about `font-size × line-height` tall; a heading of `n` lines about `size × n × 1.15`;
  - a character is about `0.55 × font-size` wide, so 28 px body text fits about 75 characters across 1152 px, and a 64 px headline about 32;
  - a bullet list of five lines at 28 px with a 1.4 line height and 16 px gaps is about 260 px;
  - a table row is about `2 × font-size` per line of text, plus its border;
  - a card is its lines of text, times size and line height, plus its padding. Leave its height out, so content sets it.
- **Size boxes to their longest word.** A word longer than its box runs out of it. In a narrow column, check the longest word in each cell.
- **Too much for 592 px? Split the slide.** Never shrink the type or the margins to fit.
- **Fill the space you use.** Content that fills about two thirds of the height reads as deliberate. Content huddled at the top of an empty slide reads as unfinished. Use `justify-content: space-between`, a centred statement, or a larger figure.

## Type

- **One scale, four or five sizes,** each a class in `style`: for example 64 (headline), 40 (subhead or big number label), 28 (body), 22 (caption), 20 (footer). A big number on its own can go to 120 or more.
- **One to three typefaces.** Emphasise with weight, italic or colour, never with a new size.
- **Headlines** get `text-wrap: balance` and a line height near 1.1. Body text near 1.4.
- **Numbers** that line up in a table or stack use `font-variant-numeric: tabular-nums`.
- Without a design system or a face the subject calls for, Inter for text and Space Grotesk for headings are good defaults.

## Colour

- **One dark, one light, one or two accents.** Prefer toned whites and blacks to `#fff` and `#000`.
- **Two background tones** give rhythm: most slides on one, section openers or a statement slide on the other or on the accent.
- **On an accent background,** check which of the dark or light text holds 4.5:1, and use that one.
- **Status colours** (good, warning, bad) mean only that, and come with a word or an icon.
- **Charts** take their series colours from `artifact-dataviz`, checked with `silt design palette`.

## Layouts worth repeating

Pick three to five for a deck and make each a class:
- **Title:** the deck's name, one line on what it's for, the presenter and the date.
- **Statement:** one sentence, large, on its own. The point of a section.
- **Evidence:** a headline that states the finding, and one chart, table or screenshot that proves it, with the source in the footer.
- **Big number:** one figure, its unit and a line saying what it means, with its source.
- **Comparison:** two or three columns that share one structure, such as before and after, or options with the same rows.
- **Cards:** two to four short items in a row, each with a short title and one or two lines.
- **Quote:** the words, who said them and where. Only real quotes the person gave you.
- **Close:** what to do next, and who does it.

Slides of one layout share one markup. Repeated elements keep their places from slide to slide, so nothing jumps as the presenter moves on.

## Charts and diagrams on a slide

- **Draw them as inline SVG** sized to the space they take on the stage, with a `viewBox` and `width: 100%`. Silt's data page charts aren't available inside a slide.
- **Labels at 20 px or more,** direct labels on the series rather than a legend when there are four or fewer.
- **One chart per slide,** and the headline says what it shows.
- **Diagrams** follow `artifact-diagramming`: the mechanism, labelled arrows, `currentColor` strokes.

## Notes

- Write what the presenter would say, in full sentences, one short paragraph per slide: the transition in, the point, the evidence in words, the transition out.
- Put the detail that didn't fit on the slide here, such as the method behind a number.
- A deck meant to be read without a speaker still gets notes for whoever presents it later, but carries its point on the slide itself.

## Before you hand it over

Check against this list, without rendering every slide:
- every slide fits its stage at the sizes above, with nothing past y 656 except a footer;
- every headline is a sentence that states the slide's point;
- no text is under 20 px;
- every figure has a source on the slide or in its notes, or is a marked placeholder;
- the look holds in both themes, or the deck commits to one.
