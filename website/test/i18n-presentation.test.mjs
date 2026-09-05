import test from 'node:test';
import assert from 'node:assert/strict';
import { navigationIdentity, localizeNavigation } from '../scripts/navigation.mjs';
import { validateTranslation } from '../scripts/i18n.mjs';
import { estimateReadingMinutes } from '../lib/reading-time.mjs';
test('navigation labels translate without changing destinations or ordering', () => {
 const source=JSON.stringify({pages:['index','---Start---','[Capabilities](/docs/capabilities)']});
 const unit={kind:'content',id:'docs/meta.json',source};
 assert.doesNotThrow(()=>validateTranslation(unit,JSON.stringify({pages:['index','---开始---','[能力](/docs/capabilities)']})));
 assert.throws(()=>validateTranslation(unit,JSON.stringify({pages:['index','---开始---','[能力](/wrong)']})));
 assert.throws(()=>validateTranslation(unit,JSON.stringify({pages:['other','---开始---','[能力](/docs/capabilities)']})));
 assert.equal(navigationIdentity('---开始---'),'---separator---');
});
test('navigation prefixes only available translated destinations', () => {
 const pages=['[能力](/docs/capabilities)','[Missing](/docs/missing)','[External](https://example.com)'];
 assert.deepEqual(localizeNavigation({pages},'zh-CN',new Set(['/docs/capabilities'])).pages,['[能力](/zh-CN/docs/capabilities)',pages[1],pages[2]]);
});
test('reading time counts unspaced Chinese and mixed prose, excluding fenced code', () => {
 assert.equal(estimateReadingMinutes('文'.repeat(2000)),5);
 assert.equal(estimateReadingMinutes('word '.repeat(1100)),5);
 assert.equal(estimateReadingMinutes('文'.repeat(800)+' word '.repeat(440)),4);
 assert.equal(estimateReadingMinutes('```ts\n'+'文'.repeat(2000)+'\n```'),1);
});
