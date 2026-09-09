const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { spawn } = require('node:child_process');
const readline = require('node:readline');
const vscode = require('vscode');

const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(description, read, accept = Boolean) {
  const deadline = Date.now() + 25000;
  let last;
  while (Date.now() < deadline) {
    last = await read();
    if (accept(last)) return last;
    await pause(150);
  }
  throw new Error(`${description}: timed out; last=${JSON.stringify(last)}`);
}
async function replace(document, text, save = true) {
  const edit = new vscode.WorkspaceEdit();
  edit.replace(document.uri, new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length)), text);
  assert(await vscode.workspace.applyEdit(edit));
  if (save) assert(await document.save());
}
async function completions(document, marker) {
  const offset = document.getText().indexOf(marker);
  assert(offset >= 0, `missing completion marker ${marker}`);
  const result = await vscode.commands.executeCommand('vscode.executeCompletionItemProvider',
    document.uri, document.positionAt(offset + marker.length));
  return (result?.items || []).map(item => typeof item.label === 'string' ? item.label : item.label.label);
}

// A deliberately small saved-document adapter. It exercises the existing event
// contract through the real DiagnosticsCollection API, not a replacement LSP.
async function diagnosticAdapter() {
  const workspace = vscode.workspace.workspaceFolders[0].uri;
  const canonicalRoot = await fs.realpath(workspace.fsPath);
  const collection = vscode.languages.createDiagnosticCollection('tysel-editor-smoke');
  let generation = -1;
  const events = [];
  let queue = Promise.resolve();
  async function apply(event) {
    if (event.event !== 'diagnostics' || event.schemaVersion !== 1 || event.generation <= generation) return;
    const grouped = new Map();
    for (const diagnostic of event.diagnostics) {
      if (!diagnostic.file) continue;
      const relative = path.relative(canonicalRoot, diagnostic.file);
      const uri = !relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative)
        ? vscode.Uri.file(path.join(workspace.fsPath, relative)) : vscode.Uri.file(diagnostic.file);
      const document = await vscode.workspace.openTextDocument(uri);
      const position = value => {
        if (!value) return new vscode.Position(0, 0); // file-level fallback only
        if (Number.isInteger(value.byteOffset)) {
          return document.positionAt(Buffer.from(document.getText()).subarray(0, value.byteOffset).toString('utf8').length);
        }
        const line = value.line - 1;
        const character = Array.from(document.lineAt(line).text).slice(0, value.column - 1).join('').length;
        return new vscode.Position(line, character);
      };
      const item = new vscode.Diagnostic(new vscode.Range(position(diagnostic.start), position(diagnostic.end)),
        diagnostic.message, diagnostic.severity === 'warning' ? vscode.DiagnosticSeverity.Warning : vscode.DiagnosticSeverity.Error);
      item.code = diagnostic.code;
      item.source = 'Tysel smoke';
      const key = uri.toString();
      if (!grouped.has(key)) grouped.set(key, [uri, []]);
      grouped.get(key)[1].push(item);
    }
    collection.clear();
    collection.set([...grouped.values()]);
    generation = event.generation;
    events.push(event);
  }
  return {
    collection, events,
    get generation() { return generation; },
    accept(event) { queue = queue.then(() => apply(event)); return queue; },
    flush() { return queue; },
  };
}

exports.run = async function run() {
  const root = vscode.workspace.workspaceFolders[0].uri.fsPath;
  const report = { timestamp: new Date().toISOString(), vscode: vscode.version, cases: [], scope: 'Real VS Code providers and a test-only saved-document diagnostic adapter; no production extension installed.' };
  const adapter = await diagnosticAdapter();
  let child;
  let streamError;
  const test = async (name, operation) => {
    const started = Date.now();
    await operation();
    report.cases.push({ name, passed: true, milliseconds: Date.now() - started });
  };
  const nextSnapshot = async operation => {
    const before = adapter.generation;
    await operation();
    await until('dev diagnostic snapshot', () => {
      if (streamError) throw streamError;
      return adapter.generation > before;
    });
  };
  try {
    const ts = vscode.extensions.getExtension('vscode.typescript-language-features');
    const toml = vscode.extensions.getExtension('tamasfe.even-better-toml');
    assert(ts && toml, 'Required editor extensions were not discovered');
    report.typescriptExtension = ts.packageJSON.version;
    report.bundledTypeScript = JSON.parse(await fs.readFile(path.join(vscode.env.appRoot, 'extensions/node_modules/typescript/package.json'), 'utf8')).version;
    report.tomlExtension = toml.packageJSON.version;
    await ts.activate();
    await toml.activate();
    const manifest = await vscode.workspace.openTextDocument(path.join(root, 'tysel.toml'));
    const baseManifest = manifest.getText();
    const source = await vscode.workspace.openTextDocument(path.join(root, 'src/index.ts'));
    const baseSource = `import type { TyselApp } from '@tysel/types';
import type { TyselEnv } from '../tysel-env.js';
export default {
  async fetch(_request, runtime) {
    runtime.sqlite;
    return new Response('ok');
  }
} satisfies TyselApp<TyselEnv>;
`;
    await replace(source, baseSource);
    await fs.writeFile(path.join(root, 'src/nested.ts'), `const label = '王😀'; import 'node:fs'; export default {};\n`);
    await fs.writeFile(path.join(root, 'src/other.ts'), `import 'node:path'; export default {};\n`);
    await test('TOML schema supplies profile completion without online catalogs', async () => {
      await vscode.window.showTextDocument(manifest);
      await replace(manifest, baseManifest.replace('profile = "service"', 'profile = ""'), false);
      const labels = await until('TOML completion', () => completions(manifest, 'profile = "'),
        items => items.some(item => item.includes('isolated')));
      assert(labels.some(item => item.includes('component')));
      await replace(manifest, baseManifest);
    });
    await test('TypeScript offers declared capabilities and resolves generated types', async () => {
      await vscode.window.showTextDocument(source);
      const labels = await until('runtime completion', () => completions(source, 'runtime.'), items => items.includes('sqlite'));
      assert(!labels.includes('fs') && !labels.includes('secrets'));
      const position = source.positionAt(source.getText().indexOf('TyselEnv') + 2);
      const definitions = await vscode.commands.executeCommand('vscode.executeDefinitionProvider', source.uri, position);
      assert(definitions.some(item => (item.targetUri || item.uri).fsPath.endsWith('tysel-env.d.ts')));
    });
    child = spawn(process.env.TYSEL_EDITOR_CLI, ['--error-format', 'json', 'dev'], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    const logs = [];
    child.stdout.on('data', data => logs.push(data.toString()));
    const lines = readline.createInterface({ input: child.stderr });
    lines.on('line', line => {
      logs.push(line);
      try { adapter.accept(JSON.parse(line)).catch(error => { streamError = error; }); } catch { /* human log */ }
    });
    child.on('error', error => { streamError = error; });
    child.on('exit', (code, signal) => { if (code !== null && code !== 0) streamError = new Error(`dev exited ${code}: ${logs.join('\n')}`); });
    await until('dev startup', () => { if (streamError) throw streamError; return adapter.generation >= 0; });
    await test('Saved permission grants refresh TypeScript completion without restarting the service', async () => {
      await nextSnapshot(() => replace(manifest, baseManifest.replace('fs_read = []', 'fs_read = ["./input"]').replace('secrets = []', 'secrets = ["TOKEN"]')));
      assert.equal(adapter.events.at(-1).diagnostics.length, 0, JSON.stringify(adapter.events.at(-1)));
      await until('new capabilities', () => completions(source, 'runtime.'), items => items.includes('fs') && items.includes('secrets'));
      assert((await fs.readFile(path.join(root, 'tysel-env.d.ts'), 'utf8')).includes('TOKEN'));
    });
    await test('Invalid manifest is shown by both TOML and Tysel providers and preserves types', async () => {
      const oldTypes = await fs.readFile(path.join(root, 'tysel-env.d.ts'), 'utf8');
      await nextSnapshot(() => replace(manifest, manifest.getText().replace('workers = 1', 'workers = 0')));
      const item = adapter.collection.get(manifest.uri)?.[0];
      assert.equal(item?.code, 'TYSEL_MANIFEST_INVALID');
      assert.equal(manifest.getText(item.range), '0');
      await until('TOML schema diagnostic', () => vscode.languages.getDiagnostics(manifest.uri),
        diagnostics => diagnostics.some(item => item.source !== 'Tysel smoke' && item.message.includes('minimum')));
      assert.equal(await fs.readFile(path.join(root, 'tysel-env.d.ts'), 'utf8'), oldTypes);
    });
    await test('Repairing and revoking grants clears manifest diagnostics and removes capabilities', async () => {
      await nextSnapshot(() => replace(manifest, baseManifest));
      assert(!adapter.collection.get(manifest.uri)?.length);
      await until('revoked capabilities', () => completions(source, 'runtime.'), items => items.includes('sqlite') && !items.includes('fs') && !items.includes('secrets'));
      await until('TOML diagnostics cleared', () => vscode.languages.getDiagnostics(manifest.uri), items => !items.length);
    });
    await test('Nested import errors convert UTF-8 offsets into correct editor UTF-16 ranges', async () => {
      await nextSnapshot(() => replace(source, `export { default } from './nested.js';\n`));
      const document = await vscode.workspace.openTextDocument(path.join(root, 'src/nested.ts'));
      const item = adapter.collection.get(document.uri)?.[0];
      assert.equal(item?.code, 'TYSEL_NODE_BUILTIN_UNSUPPORTED');
      assert(document.getText(item.range).includes('node:fs'));
      assert.equal(item.range.start.character, document.getText().indexOf("'node:fs'") + (document.getText(item.range).startsWith("'") ? 0 : 1));
      await vscode.window.showTextDocument(document, { selection: item.range });
      assert.equal(vscode.window.activeTextEditor.document.uri.toString(), document.uri.toString());
    });
    await test('Snapshots replace old-file errors and stale generations cannot restore them', async () => {
      await vscode.window.showTextDocument(source);
      const previous = adapter.events.at(-1);
      await nextSnapshot(() => replace(source, `export { default } from './other.js';\n`));
      assert(!adapter.collection.get(vscode.Uri.file(path.join(root, 'src/nested.ts')))?.length);
      assert.equal(adapter.collection.get(vscode.Uri.file(path.join(root, 'src/other.ts')))?.[0].code, 'TYSEL_NODE_BUILTIN_UNSUPPORTED');
      await adapter.accept(previous);
      assert(!adapter.collection.get(vscode.Uri.file(path.join(root, 'src/nested.ts')))?.length);
      await nextSnapshot(() => replace(source, baseSource));
      assert(!vscode.languages.getDiagnostics().some(([, items]) => items.some(item => item.source === 'Tysel smoke')));
    });
    report.passed = true;
    report.generations = adapter.events.map(event => event.generation);
  } catch (error) {
    report.passed = false;
    report.error = error.stack;
    report.lastDevEvent = adapter.events.at(-1);
    throw error;
  } finally {
    if (child && child.exitCode === null) {
      child.kill('SIGINT');
      await Promise.race([new Promise(resolve => child.once('exit', resolve)), pause(2000)]);
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    }
    await adapter.flush().catch(() => {});
    adapter.collection.dispose();
    await fs.writeFile(process.env.TYSEL_EDITOR_REPORT, JSON.stringify(report, null, 2) + '\n');
  }
};
