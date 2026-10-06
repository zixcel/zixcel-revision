from pathlib import Path
import hashlib, json, os, re, subprocess, sys, tarfile, tomllib

mode, folder = sys.argv[1:]
directory = Path(folder)
name, version = 'zixcel-revision', '0.10.0'
filename = name + '-' + version + '.crate'
sha = os.environ['GITHUB_SHA']
assert re.fullmatch(r'[a-f0-9]{40}', sha)
assert os.environ['GITHUB_REPOSITORY'] == 'zixcel/zixcel-revision'
archive = directory / filename
assert archive.is_file() and not archive.is_symlink() and archive.stat().st_size < 10_000_000
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
with tarfile.open(archive, 'r:gz') as tf:
    members = tf.getmembers()
    assert len(members) < 1000 and all(m.isfile() and m.size < 2_000_000 for m in members)
    names = [m.name for m in members]
    assert len(names) == len(set(names))
    prefix = name + '-' + version + '/'
    assert all(n.startswith(prefix) and '..' not in Path(n).parts for n in names)
    files = {m.name[len(prefix):]: tf.extractfile(m).read() for m in members}
    tracked = set(subprocess.check_output(['git', 'ls-tree', '-r', '-z', '--name-only', sha]).decode().split('\0'))
    forbidden = {'.git', 'node_modules', 'target', 'dist', 'build', 'registration', 'state', '.state', '.data', 'secrets', 'credentials'}
    for path, raw in files.items():
        assert not (set(Path(path).parts) & forbidden), f'Forbidden archive path: {path}'
        if path not in ('Cargo.toml', 'Cargo.toml.orig', '.cargo_vcs_info.json'):
            assert path in tracked, f'Archive input is not tracked at approved SHA: {path}'
            assert raw == subprocess.check_output(['git', 'show', f'{sha}:{path}']), f'Archive input differs from approved SHA: {path}'
    assert files['Cargo.toml.orig'] == subprocess.check_output(['git', 'show', f'{sha}:Cargo.toml'])
    manifest = tomllib.loads(files['Cargo.toml'].decode())
    assert manifest['package']['name'] == name and manifest['package']['version'] == version
    assert manifest['package']['publish'] == ['crates-io']
    assert files['Cargo.toml.orig'] == Path('Cargo.toml').read_bytes()
    vcs = json.loads(files['.cargo_vcs_info.json'])
    assert vcs['git']['sha1'] == sha and not vcs['git'].get('dirty', False)
    assert vcs.get('path_in_vcs', '') == ''
    for path in ('LICENSE', 'NOTICE'):
        assert files[path] == Path(path).read_bytes()
    for path, raw in files.items():
        assert path in ('Cargo.toml', 'Cargo.toml.orig', '.cargo_vcs_info.json') or raw == Path(path).read_bytes()
        assert not re.search(rb'gh[pousr]_[A-Za-z0-9]{20}|npm_[A-Za-z0-9]{20}|BEGIN .*PRIVATE KEY|[A-Z]:\\Users\\|/home/[^\s/]+/', raw)
    lock = tomllib.loads(files['Cargo.lock'].decode())
    assert all(p.get('source', 'registry+https://github.com/rust-lang/crates.io-index') == 'registry+https://github.com/rust-lang/crates.io-index' for p in lock['package'])
    def public_dependencies(value):
        if isinstance(value, dict):
            assert not any(k in value for k in ('registry', 'registry-index', 'path', 'git'))
            for v in value.values(): public_dependencies(v)
        elif isinstance(value, list):
            for v in value: public_dependencies(v)
    for key in ('dependencies', 'build-dependencies', 'dev-dependencies', 'target'):
        public_dependencies(manifest.get(key, {}))
record = {'source': sha, 'name': name, 'version': version, 'filename': filename, 'sha256': digest}
if mode == 'prepare':
    assert set(p.name for p in directory.iterdir()) == {filename}
    (directory / 'release.json').write_text(json.dumps(record) + '\n')
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output: output.write('archive-sha256=' + digest + '\n')
elif mode == 'verify':
    assert all(p.is_file() and not p.is_symlink() for p in directory.iterdir())
    assert json.loads((directory / 'release.json').read_text()) == record
    assert os.environ['EXPECTED_ARCHIVE_SHA256'] == digest
    assert set(p.name for p in directory.iterdir()) == {filename, 'release.json'}
    assert all(p.is_file() and not p.is_symlink() for p in directory.iterdir())
else:
    raise AssertionError('Expected prepare or verify')
print('Crate content, public dependencies and source binding verified')
