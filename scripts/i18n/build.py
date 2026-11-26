"""Turns scripts/i18n/<code>.txt (menu bar names, then `English|Translation` lines) into
crates/picasso-ui/src/i18n/<code>.rs, and checks the fit rules the menus need:

- a menu bar name is at most MENU_MAX characters (the title row has a fixed slot for each);
- an entry is at most 6 characters or a third longer than the English one (20 characters always fit: a
  menu is that wide before it has a label), and never over 48.
"""
import sys, pathlib, unicodedata
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from PIL import ImageFont

ROOT = pathlib.Path(__file__).resolve().parents[2]
HERE = ROOT / 'scripts' / 'i18n'
OUT = ROOT / 'crates' / 'picasso-ui' / 'src' / 'i18n'
ENGLISH = (HERE / 'labels.txt').read_text(encoding='utf-8').split('\n')
MENU_MAX = [5, 5, 5, 9, 5, 5, 5, 9, 7, 5, 8, 11, 8]
MENU_NAMES = ['File', 'Edit', 'Trim', 'Timeline', 'Clip', 'Mark', 'View', 'Playback',
              'Effects', 'Color', 'Audio', 'Workspace', 'Help']


FONT = ImageFont.truetype(str(ROOT / 'crates' / 'picasso-ui' / 'assets' / 'fonts' / 'NotoSans-Regular.ttf'), 14)


FONT_DIR = ROOT / 'crates' / 'picasso-ui' / 'assets' / 'fonts'
ARABIC = ImageFont.truetype(str(FONT_DIR / 'NotoSansArabic-Regular.subset.ttf'), 14, layout_engine=ImageFont.Layout.BASIC)
LATIN = ImageFont.truetype(str(FONT_DIR / 'NotoSans-Regular.ttf'), 14, layout_engine=ImageFont.Layout.BASIC)
# Languages whose script the bundled Latin font does not draw: their width is measured per character
# (a Chinese or Japanese character is as wide as it is tall; the Arabic ones come from the Arabic font),
# and their entries are held to a width in pixels, since a character count says little there.
OTHER = {'zh', 'ja', 'ar', 'hi', 'bn', 'th', 'ko'}
SHAPED = {'hi', 'bn', 'th'}
NAMES = {'hi': 'हिन्दी', 'bn': 'বাংলা', 'th': 'ไทย'}
CODE = ''
_latin_cmap = None


def shape(text):
    """Arabic is drawn letter by letter, left to right (the toolkit does not join or turn it): the text is
    joined into its presentation forms and put in the order it is seen, here, once."""
    if CODE in SHAPED:
        import devanagari
        devanagari.select(CODE)
        return devanagari.to_pua(text)
    if CODE != 'ar':
        return text
    import arabic_reshaper
    from bidi import get_display
    keep = chr(0xE0FF)  # the placeholder of a sentence must not be mirrored with the brackets
    text = text.replace('{}', keep)
    out = chr(10).join(get_display(arabic_reshaper.reshape(line)) for line in text.split(chr(10)))
    return out.replace(keep, '{}')


def width(text):
    if CODE not in OTHER:
        return FONT.getlength(text)
    global _latin_cmap
    if _latin_cmap is None:
        from fontTools.ttLib import TTFont
        _latin_cmap = set(TTFont(str(FONT_DIR / 'NotoSans-Regular.ttf')).getBestCmap())
    total = 0.0
    if CODE in SHAPED:
        import re
        import devanagari
        devanagari.select(CODE)
        for part in re.split('(' + devanagari.RUN.pattern + ')', text):
            if part and devanagari.RUN.fullmatch(part):
                total += devanagari.run_width(part)
            else:
                total += sum(LATIN.getlength(c) for c in part)
        return total
    for c in shape(text):
        if ord(c) in _latin_cmap:
            total += LATIN.getlength(c)
        elif unicodedata.east_asian_width(c) in 'WF':
            total += 14
        elif unicodedata.east_asian_width(c) == 'H':
            total += 7
        else:
            total += ARABIC.getlength(c)
    return total


# Words that have room where the pages draw them (the rows of a list): a translation may be up to half
# as wide again there. The buttons of the toolbars do not: they sit side by side.
ROOMY = {'Color Bars', 'Half', 'Eighth', 'Teal', 'Default', 'Off'}
# The page tabs along the bottom: centred on a tab 100 px wide, so a name may grow a little to both sides
# but is kept near the English (a long word beside short ones looks wrong).
TABS = {'Media', 'Photo', 'Cut', 'Edit', 'Spectra', 'Color', 'Decibel', 'Deliver'}


DIALOG = set((HERE / 'keys_dialogs.txt').read_text(encoding='utf-8').split(chr(10)))


def fits_page(english, shown):
    if english in DIALOG:
        return True  # a dialog's sentence wraps, so it may be as long as it needs
    we, ws = width_en(english), width(shown)
    # Text on a page is made smaller down to 3/4 when it is wider than the English; wider than that
    # (a third more) would run into its neighbour.
    if len(english) >= 18 and english not in ROOMY:
        # A short sentence fills the room it has (a hint in the middle of a panel): not wider than the
        # English. A long one is drawn smaller when it is wider (down to 3/4), so up to a third wider.
        if len(english) >= 40:
            return ws <= we * 1.3
        return ws <= we + 2
    if english in TABS:
        return ws <= we * 1.4 + 10
    if english in ROOMY:
        return ws <= we * 1.5 + 8
    return ws <= we * 1.15 + 4


def entry_max(english):
    return min(max(len(english) + 6, -(-len(english) * 135 // 100), 20), 48)


def clen(text):
    """Characters as the toolkit sets them: a word of Hindi is one."""
    return len(shape(text)) if CODE in SHAPED else len(text)


def entry_px(english):
    return min(max((len(english) + 6) * 7.0, width_en(english) * 1.35, 140), 336)


def width_en(text):
    return LATIN.getlength(text)


def build(code):
    global CODE
    CODE = code
    lines = (HERE / f'{code}.txt').read_text(encoding='utf-8').split('\n')
    assert lines[0] == '#menus', code
    i = lines.index('#entries')
    u = lines.index('#ui') if '#ui' in lines else len(lines)
    menus = [l for l in lines[1:i] if l]
    assert len(menus) == 13, (code, len(menus))
    problems = []
    for name, short, most in zip(MENU_NAMES, menus, MENU_MAX):
        if clen(short) > most:
            problems.append(f'menu {name} -> {short!r} is {clen(short)} > {most}')
        if code in OTHER and width(short) > most * 7.2:
            problems.append(f'menu {name} -> {short!r} is {width(short):.0f}px > {most * 7.2:.0f}px')
    pairs = []
    seen = set()
    entry_lines = list(lines[i + 1:u])
    for extra in sorted(HERE.glob(f'{code}.entries*.txt')):
        entry_lines += extra.read_text(encoding='utf-8').split('\n')
    latest = {}
    for l in entry_lines:
        if '|' in l:
            en, tr = l.split('|', 1)
            latest[en] = tr
    for en, tr in latest.items():
        assert en in ENGLISH, (code, en)
        seen.add(en)
        if tr != en and clen(tr) > entry_max(en):
            problems.append(f'{en!r} -> {tr!r} is {clen(tr)} > {entry_max(en)}')
        if code in OTHER and tr != en and width(tr) > entry_px(en):
            problems.append(f'{en!r} -> {tr!r} is {width(tr):.0f}px > {entry_px(en):.0f}px')
        pairs.append((en, tr))
    missing = [e for e in ENGLISH if e not in seen]
    ui = []
    ui_lines = []
    for extra in sorted(HERE.glob(f'{code}.ui*.txt')):
        ui_lines += extra.read_text(encoding='utf-8').split('\n')
    if not ui_lines:
        ui_lines = lines[u + 1:]
    latest_ui = {}
    for l in ui_lines:
        if '|' in l:
            en, tr = l.split('|', 1)
            latest_ui[en] = tr
    ui_lines = [f'{en}|{tr}' for en, tr in latest_ui.items()]
    collected = set((HERE / 'ui_labels.txt').read_text(encoding='utf-8').split('\n'))
    collected |= set((HERE / 'ui_more.txt').read_text(encoding='utf-8').split('\n'))
    on_pages = set(collected)
    collected |= set(ENGLISH)
    for l in ui_lines:
        if not l.strip():
            continue
        en, tr = l.split('|', 1)
        if not fits_page(en, tr):
            problems.append(f'ui text {en!r} -> {tr!r} is {width(tr):.0f}px, the English {width_en(en):.0f}px')
        if en not in collected:
            problems.append(f'ui text {en!r} is not shown anywhere (scripts/i18n/ui_labels.txt)')
        ui.append((en, tr))
    # A menu word that is also text on a page (Cut, Edit, Media...) is drawn there too, so it has to fit
    # there, unless the page text has a short form of its own.
    own = {en for en, _ in ui}
    for en, tr in latest.items():
        if en in on_pages and en not in own and not fits_page(en, tr):
            problems.append(f'page text {en!r} -> {tr!r} is {width(tr):.0f}px, the English {width_en(en):.0f}px (from the menus)')
    return menus, pairs, ui, problems, missing


def rust(code, menus, pairs, ui):
    global CODE
    CODE = code
    menus = [shape(m) for m in menus]
    pairs = [(e, shape(t)) for e, t in pairs]
    # The version changes with every release: its line is a sentence with the number left out.
    ui = [(('Version {}', t.replace('0.6.1', '{}').replace('০.৬.১', '{}')) if e == 'Version 0.6.1' else (e, t)) for e, t in ui]
    ui = [(e, shape(t)) for e, t in ui]
    esc = lambda s: s.replace(chr(92), chr(92) * 2).replace(chr(34), chr(92) + chr(34))
    out = [f'//! The menus in {code}. Made from scripts/i18n/{code}.txt by scripts/i18n/build.py.', '']
    out.append('pub(super) const MENUS: [&str; 13] = [')
    out += [f'    "{esc(m)}",' for m in menus]
    out.append('];\n')
    out.append('pub(super) const ENTRIES: &[(&str, &str)] = &[')
    out += [f'    ("{esc(e)}", "{esc(t)}"),' for e, t in pairs]
    out.append('];\n')
    out.append('pub(super) const UI: &[(&str, &str)] = &[')
    out += [f'    ("{esc(e)}", "{esc(t)}"),' for e, t in ui]
    out.append('];')
    return '\n'.join(out) + '\n'


if __name__ == '__main__':
    bad = False
    OUT.mkdir(parents=True, exist_ok=True)
    for code in sys.argv[1:]:
        menus, pairs, ui, problems, missing = build(code)
        print(code, len(pairs), 'entries,', len(missing), 'not translated,', len(ui), 'texts of the pages')
        if code in SHAPED:
            import devanagari
            devanagari.select(code)
            devanagari.to_pua(NAMES[code])
        for p in problems:
            print('  TOO LONG:', p)
            bad = True
        (OUT / f'{code}.rs').write_text(rust(code, menus, pairs, ui), encoding='utf-8', newline='\n')
    for code in sys.argv[1:]:
        if code in SHAPED:
            import devanagari
            devanagari.select(code)
            print('  font of', devanagari.write_font(str(FONT_DIR / devanagari.SCRIPTS[code]['out'])), 'words written')
    sys.exit(1 if bad else 0)
