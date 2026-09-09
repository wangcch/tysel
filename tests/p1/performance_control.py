"""Interleave same-binary A/A and before/after A/B macOS performance controls.

Uses performance_ab.py's completion-checked, fresh-process SQLite workload.
Example: python3 tests/p1/performance_control.py --root /tmp/new-control \
    --before-bin /path/to/baseline --after-bin /path/to/candidate
Evidence must be retained even when a subsequent batch reverses direction.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import statistics


def summarize(rows):
    output = {}
    for condition in ('AA', 'AB'):
        grouped = {v: sorted((r for r in rows if r['condition'] == condition and r['version'] == v),
                             key=lambda r: r['block']) for v in ('before', 'after')}
        assert grouped['before'] and len(grouped['before']) == len(grouped['after'])
        assert [r['block'] for r in grouped['before']] == [r['block'] for r in grouped['after']]
        if condition == 'AA':
            assert len({r['binary_sha256'] for v in grouped.values() for r in v}) == 1
        metrics = {}
        for name in ('throughput_s', 'p95_ms', 'p99_ms', 'cpu_ms_per_op',
                     'cpu_core_percent', 'rss_peak_mib', 'database_storage_write_bytes_per_op'):
            b, a = ([r[name] for r in grouped[v]] for v in ('before', 'after'))
            bm, am = statistics.median(b), statistics.median(a)
            metrics[name] = {'before_median': bm, 'after_median': am,
                             'median_ratio_delta_percent': (am / bm - 1) * 100 if bm else None,
                             'before_range': [min(b), max(b)], 'after_range': [min(a), max(a)],
                             'paired_delta_percent': [(y / x - 1) * 100 if x else None for x, y in zip(b, a)]}
        output[condition] = metrics
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--before-bin', type=Path)
    parser.add_argument('--after-bin', type=Path)
    parser.add_argument('--seconds', type=float, default=15)
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--summarize-only', action='store_true')
    args = parser.parse_args()
    root = args.root.resolve()
    result_path = root / 'measurements/results.json'
    if args.summarize_only:
        rows = json.loads(result_path.read_text())
    else:
        if not args.before_bin or not args.after_bin or args.seconds <= 0 or args.repeats < 1:
            parser.error('provide both binary directories, positive seconds and repeats')
        if (root / 'measurements').exists():
            parser.error('use a fresh root to preserve earlier measurements')
        root.mkdir(parents=True, exist_ok=True)
        old, new = args.before_bin.resolve(), args.after_bin.resolve()
        for directory in (old, new):
            assert (directory / 'tysel').is_file() and (directory / 'tysel-worker').is_file()
        os.environ['TYSEL_AB_ROOT'] = str(root)
        os.environ['TYSEL_AB_SECONDS'] = str(args.seconds)
        harness = runpy.run_path(str(Path(__file__).with_name('performance_ab.py')))
        context = harness['Service'].__init__.__globals__
        rows = []
        for block in range(args.repeats):
            for condition in (('AA', 'AB') if block % 2 == 0 else ('AB', 'AA')):
                context['BEFORE'] = new if condition == 'AA' else old
                context['AFTER'] = new
                for version in (('before', 'after') if block % 2 == 0 else ('after', 'before')):
                    binary = (context['BEFORE'] if version == 'before' else new) / 'tysel'
                    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
                    row = harness['phase'](version, 'one', 1, str(block) + '-' + condition)
                    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
                    row.update(condition=condition, block=block, binary_sha256=digest)
                    rows.append(row)
                    result_path.write_text(json.dumps(rows, indent=2) + '\n')
                    print(json.dumps(row), flush=True)
    output = summarize(rows)
    (root / 'control-summary.json').write_text(json.dumps(output, indent=2) + '\n')
    print(json.dumps(output, indent=2))


if __name__ == '__main__':
    main()
