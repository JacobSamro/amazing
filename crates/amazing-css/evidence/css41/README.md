CSS41 actual first-font metric context, fixed CSS40 da0e41f0 base.

CSS owns a FontMetricContext interface without a text dependency. Full,
animated, pseudo and incremental subtree computation resolve FromFont before
inheritance. Inherited numeric aspects remain fixed when descendants change
fonts. A missing actual first font resolves FromFont to none; font-dependent
numeric math waits for contextual metrics rather than fabricated em guesses.
Uninstalled context retains the existing symbolic values explicitly.

TextProvider offers a CSS-independent inherent font_size_adjust_ratio query.
U+0030 width and U+6C34 width/height are physical horizontal/vertical advances,
independent of paragraph writing-mode and text-orientation. Existing dynamic
TextSystem adjusted_font_size reuses exactly these ratios. Missing glyphs use
standard metric fallbacks; missing actual fonts return None.

Driver installation/loading before CSS computation belongs to the root lane.
This ticket proves the complete lower API and physical provider behavior; it
makes no installed-browser FromFont claim before that adapter lands.

Final Linux workspace3550/17skip+4doctests (six grouped regressions added); initial CSS+provideriteration490/2skip. Parsing4126/5126 unchanged/lost0. Wholefonts24/32 unchanged/lost0. Renderer336901f3cfdca9e9853e7465b12b8bec9381154c641d95420c5622751be6feeb.
