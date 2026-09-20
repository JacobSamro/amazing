"""Offline TEST-ONLY certificate data generation; no production trust change.
Python cryptography49.0.0. Keys are public test fixtures, never real identities.
"""
from pathlib import Path
from datetime import datetime, timezone
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID, ExtendedKeyUsageOID
import ipaddress, hashlib, json
root=Path(__file__).parent
ca_key=ec.generate_private_key(ec.SECP256R1());leaf_key=ec.generate_private_key(ec.SECP256R1())
ca_name=x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'Amazing NET-7 TEST ONLY CA')])
start=datetime(2020,1,1,tzinfo=timezone.utc);end=datetime(2099,1,1,tzinfo=timezone.utc)
ca=x509.CertificateBuilder().subject_name(ca_name).issuer_name(ca_name).public_key(ca_key.public_key()).serial_number(7001).not_valid_before(start).not_valid_after(end).add_extension(x509.BasicConstraints(ca=True,path_length=0),True).add_extension(x509.KeyUsage(False,False,False,False,False,True,True,False,False),True).sign(ca_key,hashes.SHA256())
leaf=x509.CertificateBuilder().subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'localhost')])).issuer_name(ca_name).public_key(leaf_key.public_key()).serial_number(7002).not_valid_before(start).not_valid_after(end).add_extension(x509.BasicConstraints(ca=False,path_length=None),True).add_extension(x509.SubjectAlternativeName([x509.DNSName('localhost'),x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]),False).add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]),False).sign(ca_key,hashes.SHA256())
data={'ca.der':ca.public_bytes(serialization.Encoding.DER),'server.der':leaf.public_bytes(serialization.Encoding.DER),'server-key.der':leaf_key.private_bytes(serialization.Encoding.DER,serialization.PrivateFormat.PKCS8,serialization.NoEncryption())}
for name,b in data.items():(root/name).write_bytes(b)
(root/'pin.json').write_text(json.dumps({n:hashlib.sha256(b).hexdigest() for n,b in data.items()},indent=2)+'\n')
