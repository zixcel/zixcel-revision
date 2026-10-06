import json, os, re, subprocess, tomllib, urllib.error, urllib.parse, urllib.request
from pathlib import Path

expected_repo = 'zixcel/zixcel-revision'
expected_name = 'zixcel-revision'
expected_version = '0.10.0'
mode = os.environ['RELEASE_MODE']
sha = os.environ['GITHUB_SHA']
assert mode in ('prepare', 'bootstrap', 'oidc')
assert os.environ['GITHUB_REPOSITORY'] == expected_repo
assert re.fullmatch(r'[a-f0-9]{40}', sha)
assert os.environ.get('APPROVED_SHA') == sha
assert os.environ.get('GITHUB_REF_TYPE') == 'tag'
assert os.environ.get('GITHUB_REF_NAME') == 'v' + expected_version
event = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text())
assert event['repository']['private'] is False
assert event['repository']['full_name'] == expected_repo
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip() == sha
subprocess.run(['git', 'merge-base', '--is-ancestor', sha, 'origin/main'], check=True)
assert not subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=no']).strip()
package = tomllib.loads(Path('Cargo.toml').read_text())['package']
assert package.get('publish') == ['crates-io']
url = 'https://crates.io/api/v1/crates/' + expected_name
assert package['name'] == expected_name and package['version'] == expected_version
assert not Path('.cargo').exists() and not Path('build.rs').exists()
assert not package.get('build')
manifest = tomllib.loads(Path('Cargo.toml').read_text())
assert manifest.get('workspace', {}).get('members', ['.']) == ['.']
if mode != 'prepare':
    assert os.environ.get('PUBLISH_ENABLED') == 'true'
    if mode == 'bootstrap':
        assert os.environ.get('BOOTSTRAP_ENABLED') == 'true'
    req = urllib.request.Request(url, headers={'User-Agent': expected_repo + ' release-preflight'})
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            metadata = json.load(response)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        metadata = None
    if mode == 'bootstrap':
        assert metadata is None, 'Name already registered; inspect before retry'
    else:
        assert metadata is not None, 'OIDC requires an existing package and configured trust'
        versions = metadata.get('versions', {})
        exists = expected_version in versions if isinstance(versions, dict) else any(v['num'] == expected_version for v in versions)
        assert not exists, 'Version already exists; inspect registry metadata/integrity'
print('Approved source, identity and release mode verified')
