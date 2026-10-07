#!/usr/bin/env python3
"""Prepare ignored build-output links only on an authenticated actual Q4."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import tempfile

from common import config, dump, read_json, require
from run_acceptance import execution_helpers, identity


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', required=True)
    parser.add_argument('--sha', required=True)
    args = parser.parse_args()
    c = config(args.config)
    output = Path(c['output_dir'])
    require(not (output / 'acceptance-environment-layout.json').exists(), 'prior layout record must not be overwritten')
    require(not (output / 'execution-helper-freeze.json').exists(), 'prior execution helper freeze must not be overwritten')
    publication = read_json(output / 'publication.json')
    before = identity(c, publication, args.sha)
    repo = Path(c['repo_dir']).resolve()
    target = repo / 'target'
    require(not target.is_symlink(), 'whole target must never be an external symlink')
    target.mkdir(exist_ok=True)
    require(target.resolve() == target and target.parent == repo, 'target physical path escaped the candidate')
    shared = Path(c['cargo_target_dir']).resolve()
    require(shared != target and shared.is_dir(), 'expected separate session-owned Cargo target')
    links = []
    for name in ('doc', 'debug'):
        link = target / name
        destination = shared / name
        require(destination.is_dir(), f'missing shared build-output directory: {destination}')
        if link.is_symlink():
            require(link.resolve() == destination, f'existing link targets another build: {link}')
        else:
            require(not link.exists(), f'existing candidate build output will not be replaced: {link}')
            link.symlink_to(destination, target_is_directory=True)
        subprocess.run(['git', 'check-ignore', '--quiet', str(link)], cwd=repo, check=True)
        links.append({'name': name, 'logical_path': str(link), 'link_text': os.readlink(link), 'physical_path': str(link.resolve()), 'git_ignored': True})
    environment = dict(os.environ)
    environment.update(PATH=c['cargo_bin_dir'] + ':' + environment.get('PATH', ''), RUSTUP_TOOLCHAIN=c['rust_toolchain'], CARGO_TARGET_DIR=c['cargo_target_dir'])
    with tempfile.TemporaryDirectory(prefix='q4-cargo-root-check-', dir=target) as temporary:
        child = Path(temporary).resolve()
        require(target in child.parents, 'preflight child workdir is not physically under the candidate')
        command = ['cargo', 'locate-project', '--message-format', 'plain']
        located = subprocess.check_output(command, cwd=child, env=environment, text=True).strip()
        require(Path(located).resolve() == repo / 'Cargo.toml', 'child Cargo discovery selected a different source tree')
        cargo_probe = {'command': command, 'cwd_physical_path': str(child), 'located_manifest': located}
    after = identity(c, publication, args.sha)
    require(before == after, 'source/body identity changed during environment preparation')
    dump(output / 'acceptance-environment-layout.json', {
        'candidate_identity': before, 'candidate_physical_root': str(repo),
        'target_is_real_directory': True, 'target_physical_path': str(target.resolve()),
        'shared_target_physical_path': str(shared), 'links': links,
        'cargo_child_discovery': cargo_probe, 'source_body_identity_unchanged': True,
        'scope': 'ignored environment preparation and Cargo project discovery only; no compile or acceptance gate executed',
    })
    dump(output / 'execution-helper-freeze.json', {
        'candidate_identity': before, 'helpers': execution_helpers(c, args.config),
        'scope': 'exact candidate helper/configuration freeze before any acceptance gate',
    })
    print(output / 'acceptance-environment-layout.json')


if __name__ == '__main__':
    main()
