"""Audit sitemap pages and multilingual SEO in the static export."""
import json
from pathlib import Path
from html.parser import HTMLParser
from urllib.parse import urlsplit
from xml.etree import ElementTree as ET
ROOT=Path(__file__).resolve().parents[1]/'out'
class Page(HTMLParser):
 def __init__(self,text):
  super().__init__();self.meta={};self.links=[];self.ld=[];self.capture=False;self.buf='';self.title='';self.intitle=False;self.h1=0;self.lang='';self.feed(text)
 def handle_starttag(self,t,a):
  d=dict(a)
  if t=='html':self.lang=d.get('lang','')
  if t=='meta':self.meta[d.get('name',d.get('property'))]=d.get('content')
  if t=='link':self.links.append(d)
  if t=='title':self.intitle=True
  if t=='h1':self.h1+=1
  if t=='script' and d.get('type')=='application/ld+json':self.capture=True;self.buf=''
 def handle_data(self,s):
  if self.capture:self.buf+=s
  if self.intitle:self.title+=s
 def handle_endtag(self,t):
  if t=='title':self.intitle=False
  if t=='script' and self.capture:self.ld.append(json.loads(self.buf));self.capture=False
urls=[e.text for e in ET.parse(ROOT/'sitemap.xml').iter('{http://www.sitemaps.org/schemas/sitemap/0.9}loc')]
assert len(urls)==len(set(urls)),'Duplicate sitemap URLs'
pages={};errors=[]
for url in urls:
 path=urlsplit(url).path;f=ROOT/path.lstrip('/')/'index.html'
 if not f.exists():errors.append(f'{url}: missing output');continue
 p=Page(f.read_text());pages[url]=p
 if not p.title or not p.meta.get('description'):errors.append(f'{url}: missing title/description')
 if p.h1!=1:errors.append(f'{url}: H1 count {p.h1}')
 if 'noindex' in p.meta.get('robots',''):errors.append(f'{url}: noindex in sitemap')
 if [x.get('href') for x in p.links if x.get('rel')=='canonical']!=[url]:errors.append(f'{url}: canonical mismatch')
 if '/docs/' in path or '/reference/' in path or '/blog/' in path or path in ['/','/zh-CN/']:
  if not p.ld:errors.append(f'{url}: missing structured data')
  for data in p.ld:
   for entity in data.get('@graph',[data]):
    if 'inLanguage' in entity and entity['inLanguage']!=p.lang:errors.append(f'{url}: JSON-LD language mismatch')
for url,p in pages.items():
 alts={x['hreflang']:x['href'] for x in p.links if x.get('hreflang')}
 if alts.get(p.lang)!=url:errors.append(f'{url}: missing self hreflang')
 for lang,target in alts.items():
  if target not in pages:errors.append(f'{url}: alternate absent from sitemap: {target}');continue
  reciprocal={x.get('hreflang'):x.get('href') for x in pages[target].links}
  if reciprocal.get(p.lang)!=url:errors.append(f'{url}: non-reciprocal hreflang')
assert not errors,'\n'.join(errors)
print(f'SEO passed: {len(pages)} sitemap pages, titles/descriptions, H1, canonicals, reciprocal hreflang and JSON-LD')
