"""Validate the built locale pages, including stable section links (run after build)."""
import json
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'out'

class Page(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.ids = set()
        self.links = []
        self.images = []
        self.metadata = {}
        self.stack = []
        self.bad_root = False
        self.feed(text)
        self.doctype = text.lower().startswith('<!doctype html>')

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        if tag == 'html' and self.stack:
            self.bad_root = True
        if 'id' in values:
            self.ids.add(values['id'])
        if tag == 'img':
            self.images.append(values.get('src'))
        if tag == 'meta':
            self.metadata[values.get('property', values.get('name'))] = values.get('content')
        if tag == 'a':
            self.links.append(values.get('href', ''))
        if tag not in {'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr'}:
            self.stack.append(tag)

    def handle_endtag(self, tag):
        if tag in self.stack:
            self.stack = self.stack[:len(self.stack) - 1 - self.stack[::-1].index(tag)]

cache = {}
def read(file):
    if file not in cache:
        cache[file] = Page(file.read_text())
    return cache[file]

errors = []
checked = 0
locales = json.loads((ROOT / 'locales/config.json').read_text())
for locale, config in locales['locales'].items():
    if not config['published'] or locale == locales['sourceLocale']:
        continue
    files = list((OUT / locale).rglob('*.html'))
    assert files, f'No static pages for {locale}'
    for file in files:
        page = read(file)
        if not page.doctype or page.bad_root:
            errors.append(f'{file.relative_to(OUT)}: invalid HTML document root')
        for href in page.links:
            if not (href.startswith(f'/{locale}/') or href.startswith('#')):
                continue
            url = urlsplit(href)
            if not url.fragment:
                continue
            target = OUT / url.path.lstrip('/') / 'index.html' if url.path else file
            checked += 1
            if not target.exists() or unquote(url.fragment) not in read(target).ids:
                errors.append(f'{file.relative_to(OUT)}: missing anchor {href}')
    # Blog cards must use each translated article's title, rather than source metadata.
    listing = (OUT / locale / 'blog/index.html').read_text()
    for article in (ROOT / f'locales/{locale}/content/blog').glob('*.mdx'):
        title = next(line[7:].strip().strip('"') for line in article.read_text().splitlines() if line.startswith('title: '))
        if title not in listing:
            errors.append(f'{locale}/blog: translated title absent: {title}')
        rendered = read(OUT / locale / 'blog' / article.stem / 'index.html')
        cover = next(line[7:].strip().strip('"') for line in article.read_text().splitlines() if line.startswith('cover: '))
        if cover not in rendered.images or rendered.metadata.get('og:type') != 'article':
            errors.append(f'{locale}/blog/{article.stem}: article cover or metadata absent')
        if not rendered.metadata.get('article:published_time') or not rendered.metadata.get('author'):
            errors.append(f'{locale}/blog/{article.stem}: author or publication metadata absent')
english = read(OUT / 'index.html')
if not english.doctype or english.bad_root:
    errors.append('English home: invalid HTML document root')
assert not errors, '\n'.join(errors)
print(f'Localized HTML passed: {checked} section links, document roots and translated blog titles')
