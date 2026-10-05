"""Generate an independent TEST-ONLY validation corpus with cryptography 49.0.0.
Original NET-7 fixtures are preserved. Private keys here are public test data.
"""
from pathlib import Path
from datetime import datetime, timezone
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID, ExtendedKeyUsageOID
import hashlib, ipaddress, json
root = Path(__file__).parent / 'validation'
root.mkdir(exist_ok=True)
start = datetime(2020, 1, 1, tzinfo=timezone.utc)
end = datetime(2099, 1, 1, tzinfo=timezone.utc)
key = ec.generate_private_key(ec.SECP256R1())
ca_key = ec.generate_private_key(ec.SECP256R1())
intermediate_key = ec.generate_private_key(ec.SECP256R1())
def name(value):
    return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, value)])
ca_name = name('Amazing TLS validation TEST ONLY CA')
intermediate_name = name('Amazing TLS validation TEST ONLY intermediate')
def builder(subject, issuer, public, serial, before=start, after=end):
    return x509.CertificateBuilder().subject_name(subject).issuer_name(issuer).public_key(public).serial_number(serial).not_valid_before(before).not_valid_after(after)
ca = builder(ca_name, ca_name, ca_key.public_key(), 8001).add_extension(x509.BasicConstraints(ca=True,path_length=1),True).add_extension(x509.KeyUsage(False,False,False,False,False,True,True,False,False),True).sign(ca_key,hashes.SHA256())
intermediate = builder(intermediate_name, ca_name, intermediate_key.public_key(), 8002).add_extension(x509.BasicConstraints(ca=True,path_length=0),True).add_extension(x509.KeyUsage(False,False,False,False,False,True,True,False,False),True).sign(ca_key,hashes.SHA256())
certs = {'ca.der': ca, 'intermediate.der': intermediate}
def leaf(label, serial, sans, **options):
    issuer = intermediate_name if options.get('intermediate') else ca_name
    signer = intermediate_key if options.get('intermediate') else ca_key
    b = builder(name('localhost'),issuer,key.public_key(),serial,options.get('before',start),options.get('after',end)).add_extension(x509.BasicConstraints(ca=False,path_length=None),True)
    if sans is not None:
        b = b.add_extension(x509.SubjectAlternativeName(sans),False)
    b = b.add_extension(x509.ExtendedKeyUsage([options.get('eku',ExtendedKeyUsageOID.SERVER_AUTH)]),False)
    if options.get('critical'):
        b = b.add_extension(x509.UnrecognizedExtension(x509.ObjectIdentifier('1.2.3.4.5.6'),b'\x05\x00'),True)
    certs[label + '.der'] = b.sign(signer,hashes.SHA256())
dns = [x509.DNSName('localhost')]
leaf('valid',8100,dns+[x509.IPAddress(ipaddress.ip_address('127.0.0.1')),x509.IPAddress(ipaddress.ip_address('::1'))])
leaf('expired',8101,dns,after=datetime(2021,1,1,tzinfo=timezone.utc))
leaf('future',8102,dns,before=datetime(2090,1,1,tzinfo=timezone.utc))
leaf('wrong-purpose',8103,dns,eku=ExtendedKeyUsageOID.CLIENT_AUTH)
leaf('critical',8104,dns,critical=True)
leaf('common-name-only',8105,None)
leaf('wildcard',8106,[x509.DNSName('*.example.test')])
leaf('dns-ip-only',8107,[x509.DNSName('127.0.0.1')])
leaf('chained',8108,dns,intermediate=True)
data = {n:c.public_bytes(serialization.Encoding.DER) for n,c in certs.items()}
data['key.der'] = key.private_bytes(serialization.Encoding.DER,serialization.PrivateFormat.PKCS8,serialization.NoEncryption())
# Corrupt only the ECDSA signature bytes, retaining valid certificate DER.
bad = bytearray(data['valid.der']); bad[-1] ^= 1
data['bad-signature.der'] = bytes(bad)
for n,b in data.items():
    (root/n).write_bytes(b)
(root/'pin.json').write_text(json.dumps({'generator':'cryptography 49.0.0','sha256':{n:hashlib.sha256(b).hexdigest() for n,b in data.items()}},indent=2)+'\n')
