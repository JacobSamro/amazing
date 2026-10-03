"""Pinned W3C XML TS inventory and bulk parser observations; no expected-data injection."""
import argparse, collections, hashlib, json, pathlib, re, subprocess
import xml.etree.ElementTree as ET

ARCHIVE_URL = "https://www.w3.org/XML/Test/xmlts20130923.tar.gz"
ARCHIVE_SHA256 = "9b61db9f5dbffa545f4b8d78422167083a8568c59bd1129f94138f936cf6fc1f"

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def inventory(root):
    manifest = root / "xmlconf.xml"
    metadata = re.findall(r'<!ENTITY\s+[\w-]+\s+SYSTEM\s+"([^"]+)"', manifest.read_text())
    records = []
    for relative in metadata:
        path = root / relative
        # Some official metadata entities contain sibling TESTCASES/TEST roots.
        # This wrapper is for metadata ONLY; fixture bytes are never rewritten.
        source = re.sub(r'<\?xml\s.*?\?>', '', path.read_text(), flags=re.S)
        document = ET.fromstring('<inventory>' + source + '</inventory>')
        assert not any('xml:base' in key or key == '{http://www.w3.org/XML/1998/namespace}base'
                       for node in document.iter() for key in node.attrib), relative
        for node in document.iter('TEST'):
            attrs = dict(node.attrib)
            attrs.setdefault('RECOMMENDATION', 'XML1.0')
            attrs.setdefault('ENTITIES', 'none')
            attrs.setdefault('NAMESPACE', 'yes')
            fixture = path.parent / attrs['URI']
            assert fixture.is_file(), fixture
            recommendation = attrs['RECOMMENDATION']
            reason = None
            if recommendation in ('XML1.1', 'NS1.1') or ('VERSION' in attrs and '1.0' not in attrs['VERSION'].split()):
                reason = 'XML1.1 profile is not implemented'
            elif 'EDITION' in attrs and '5' not in attrs['EDITION'].split():
                reason = 'fixture applies only to earlier XML1.0 editions'
            elif attrs['NAMESPACE'] == 'no':
                reason = 'non-namespace-aware parser profile is not implemented'
            records.append(dict(id=attrs['ID'], fixture=str(fixture.relative_to(root)),
                                source_sha256=digest(fixture), metadata=relative,
                                metadata_sha256=digest(path), attributes=attrs,
                                description=' '.join(''.join(node.itertext()).split()),
                                profile_unsupported=reason))
    assert len(records) == 2377, len(records)
    assert len({row['id'] for row in records}) == len(records)
    return dict(archive_url=ARCHIVE_URL, archive_sha256=ARCHIVE_SHA256,
                manifest_sha256=digest(manifest), records=records)

def run(root, manifest, binary):
    eligible = [row for row in manifest['records'] if not row['profile_unsupported']]
    for row in manifest['records']:
        assert digest(root / row['fixture']) == row['source_sha256'], row['id']
    protocol = ''.join(row['id'] + '\t' + str(root / row['fixture']) + '\n' for row in eligible)
    process = subprocess.run([str(binary)], input=protocol, text=True, capture_output=True, timeout=120)
    assert process.returncode == 0, process.stderr
    lines = process.stdout.splitlines()
    assert len(lines) == len(eligible), (len(lines), len(eligible), process.stderr)
    observations = {}
    for row, line in zip(eligible, lines):
        identifier, status, message = line.split('\t')
        assert identifier == row['id']
        message = bytes.fromhex(message).decode('utf-8')
        if status in ('Unsupported', 'Limit') or (status == 'decode-error' and message.startswith('unsupported')):
            classification = 'unsupported'
        elif status in ('io-error', 'crash'):
            classification = status
        elif row['attributes']['TYPE'] == 'error':
            classification = 'optional-error-observation'
        else:
            expected_accept = row['attributes']['TYPE'] in ('valid', 'invalid')
            classification = 'pass' if (status == 'accepted') == expected_accept else 'fail'
        observations[identifier] = dict(status=status, message=message, classification=classification)
    records = [dict(row, observation=observations.get(row['id'],
                      dict(classification='profile-unsupported', message=row['profile_unsupported'])))
               for row in manifest['records']]
    return dict(archive_sha256=manifest['archive_sha256'], binary_sha256=digest(binary),
                manifest_sha256=hashlib.sha256(json.dumps(manifest,sort_keys=True).encode()).hexdigest(),
                counts=dict(collections.Counter(row['observation']['classification'] for row in records)),
                output_canonicalization_checked=False, records=records)

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=pathlib.Path)
    parser.add_argument('--manifest', type=pathlib.Path, required=True)
    parser.add_argument('--binary', type=pathlib.Path)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    if args.binary:
        assert args.output
        result = run(args.root, json.loads(args.manifest.read_text()), args.binary)
        args.output.write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps(result['counts'],sort_keys=True))
    else:
        result = inventory(args.root)
        args.manifest.write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps(dict(total=len(result['records']), profile_unsupported=sum(bool(row['profile_unsupported']) for row in result['records']))))
