"""Makes Bosnian and Serbian from the Croatian files: a few words differ (Bosnian), and Serbian is
the same in the ekavian forms, written in Cyrillic. Run: python derive.py"""
import re
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
BS = [('Tipkovnički prečaci', 'Prečice tastature'), ('zaslon', 'ekran'), ('Spremi', 'Sačuvaj'), ('spremi', 'sačuvaj'), ('sustav', 'sistem'), ('Sustav', 'Sistem'),
      ('Postavke sustava', 'Postavke sistema')]
SR = BS + [('vrijeme', 'vreme'), ('Vrijeme', 'Vreme'), ('Sljedeći', 'Sledeći'), ('sljedeći', 'sledeći'), ('Sljedeća', 'Sledeća'),
           ('lijevo', 'levo'), ('Uvezi', 'Uvezi'), ('Nađi', 'Nađi'), ('Redoslijed', 'Redosled'), ('Redosl.', 'Redos.'),
           ('rezolucij', 'rezolucij'), ('Svjetl', 'Svetl'), ('svjetl', 'svetl'), ('Mjerač', 'Merač'), ('mjesto', 'mesto'),
           ('Prostor', 'Prostor'), ('Cijel', 'Cel'), ('cijel', 'cel'), ('Pohrana', 'Skladište'), ('riječi', 'reči'),
           ('Riječi', 'Reči'), ('Ključne r.', 'Ključne r.'), ('Nema što vidjeti', 'Nema šta da se vidi'), ('Pomoć', 'Pomoć'),
           ('Ponovno', 'Ponovo'), ('ponovno', 'ponovo'), ('Ponovo', 'Ponovo'), ('Izlaz', 'Izlaz'), ('Isporuka', 'Isporuka')]
KEEP = set('''Gain Gamma Master Pan Lift Spline Scope Scopes Scopeovi scopeove scopeovi Sat Temp Tint Decibel Spectra Picasso Suite
OpenFX Cinemascope visionOS Flat Pass Proxy proxy Parade LUT EQ ADR FX XML EDL AAF Rust Windows Linux macOS Explorer Exploreru
timecode timecodea timecodeu Timecode Render render Bounce bounce Slip Slide Ripple ripple Pivot Soft Clip Esc GPL cache cachea
debayer Debayer multicam Multicam Lightbox still stillova stillove Still Stillova Reset reset Auto Jog jog scrubbing Scrubbing
Bus Preset Format Video video Foto Album Monitor Index Generator Fade fade Mix mix Dip Dissolve Sepia Mosaic Safe Area bin Bin'''.split())
BASE = {
    'Dž': 'Џ', 'DŽ': 'Џ', 'dž': 'џ', 'Lj': 'Љ', 'LJ': 'Љ', 'lj': 'љ', 'Nj': 'Њ', 'NJ': 'Њ', 'nj': 'њ',
}
LAT = dict(zip('AaBbVvGgDdĐđEeŽžZzIiJjKkLlMmNnOoPpRrSsTtĆćUuFfHhCcČčŠš',
               'АаБбВвГгДдЂђЕеЖжЗзИиЈјКкЛлМмНнОоПпРрСсТтЋћУуФфХхЦцЧчШш'))


def cyr(text):
    out = []
    for token in re.split(r'(\s+|[/:,.()+–-]+)', text):
        letters = re.sub(r'[^A-Za-zČčĆćŽžŠšĐđ]', '', token)
        if (not token or token in KEEP or re.search(r'\d|_', token) or
                re.search(r'[A-Z]{2,}', token) or re.search(r'[a-z][A-Z]', token) or
                token.rstrip('.') in KEEP or not letters):
            out.append(token)
            continue
        s = token
        for k, v in BASE.items():
            s = s.replace(k, v)
        out.append(''.join(LAT.get(c, c) for c in s))
    return ''.join(out)


def sub(text, pairs):
    text = re.sub(r'([Pp])rojekt', r'rojekat', text)
    for a, b in pairs:
        text = text.replace(a, b)
    return text


def right(line, f):
    if '|' not in line:
        return line
    n, t = line.split('|', 1)
    return n + '|' + f(t)


def make(code, pairs, translit):
    f = (lambda t: cyr(sub(t, pairs))) if translit else (lambda t: sub(t, pairs))
    nums = (HERE / 'hr.nums').read_text(encoding='utf-8').split('\n')
    out, section = [], None
    for line in nums:
        if line in ('#menus', '#entries', '#ui'):
            section = line
            out.append(line)
        elif section == '#menus':
            out.append(f(line))
        else:
            out.append(right(line, f))
    (HERE / f'{code}.nums').write_text('\n'.join(out), encoding='utf-8', newline='\n')
    for kind in ('entries2', 'ui2'):
        src = HERE / f'hr.{kind}.txt'
        lines = [right(l, f) for l in src.read_text(encoding='utf-8').split('\n')]
        (HERE / f'{code}.{kind}.txt').write_text('\n'.join(lines), encoding='utf-8', newline='\n')


if __name__ == '__main__':
    make('bs', BS, False)
    make('sr', SR, True)
