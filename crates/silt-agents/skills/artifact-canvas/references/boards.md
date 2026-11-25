# Boards: sizes, grids and states

## Sizes

| What | Board size (px) | Notes |
|---|---|---|
| Phone | 390×844 | The common width. A smaller phone is 375×667. |
| Tablet | 820×1180, or 1180×820 across | |
| Laptop window | 1280×800 | The default for a desktop app or web page. |
| Large window | 1440×900 | For dense tools and dashboards. |
| A whole web page | 1280 or 1440 wide, as tall as the page | Boards don't scroll: measure the page's height and use it. |
| A panel or dialog | its real size, such as 480×560 | Draw it over a dimmed board of the window it opens in only when that context matters. |
| A social image | 1200×630 (link preview), 1080×1080 (square), 1080×1350 (portrait) | |
| A poster | A-series ratio, such as 1240×1754 | |
| An icon set | one board per size, such as 16, 24, 48 | Show each at its real size. |

Each board's frame is its own window of that width, so a page written with CSS media queries responds to it. To show one responsive page at two widths, give both boards the same `html` and different `width`s. That shows the reflow honestly, as the product would do it.

## Grids

Positions are in canvas pixels. A row of boards reads left to right, and a grid's rows top to bottom.
- **Between boards in a row:** 80 px. The next board's `x` is the last one's `x + width + 80`.
- **Between rows:** 120 px, which leaves room for the names above the boards. The next row's `y` is the tallest board's `y + height + 120`.
- **Rows mean one thing and columns another:** states in rows and sizes in columns, or options in columns and states in rows. Say which in the boards' names and captions.
- **Align tops** within a row, and keep boards of one size in one column.
- **Keep the whole canvas a shape that fits a screen** when zoomed out: about as wide as two or three times its height. A single row of twelve boards is a strip nobody can read when it fits.

An example: a list screen in three states at two sizes.
- Row 1, y 0: `list-typical` at x 0 (1280×800), `list-typical-phone` at x 1360 (390×844).
- Row 2, y 964: `list-empty` at x 0, `list-empty-phone` at x 1360.
- Row 3, y 1928: `list-error` at x 0, `list-error-phone` at x 1360.

## The states checklist

Draw the ones that apply, not all of them:
- **Typical:** realistic data, a realistic amount of it.
- **Empty:** the first use, saying what will appear and how to add the first item.
- **Loading:** what shows while data arrives, if it takes long enough to see.
- **Long content:** the longest real name, title or number the screen can get; many items; wrapped lines.
- **Little content:** one item, a short name.
- **Error:** what went wrong and how to fix it, in the product's words.
- **Narrow:** the phone width, or a narrow panel, with menus stacked and toolbars wrapped.
- **Selected, focused, disabled:** for a control that's the point of the design.
- **Permissions:** what someone who can only read sees.

## Working prototypes

A board can run a script, so a prototype can respond to clicks within one screen: tabs that switch, a menu that opens, a form that validates. Keep it small:
- one board per screen, its script inline;
- the initial state is the one to judge, drawn complete before any click;
- every control is a real element, reachable by keyboard;
- nothing is kept between viewings, and nothing connects anywhere.

A flow across screens is a row of boards, one per step, with captions saying what the person did to get there.

## Visuals that aren't screens

Posters, social images, slides for another tool and illustrations are boards too, at the size they'll be used. Commit to one look for the set. Text holds 4.5:1 contrast over any image behind it. Put the words people will change as real text, not inside an image.
