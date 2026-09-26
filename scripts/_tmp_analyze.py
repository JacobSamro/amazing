import json,collections,sys
r=json.load(open('/home/user/amazing/out/js-intl-baseline.json'))
recs=r['records']
sel=[x for x in recs if x['status'] in ('fail','pass','unsupported')]
dir_c=collections.Counter()
for x in sel:
    p=x['path'].split('/')
    key='/'.join(p[1:3])  # intl402/NumberFormat
    dir_c[(key,x['status'])]+=1
print("=== per dir (fail/pass) ===")
for k,v in sorted(dir_c.items()):
    pass
dirs=sorted({k for k,_ in dir_c})
for d in dirs:
    print(f"{d:55s} fail={dir_c.get((d,'fail'),0):5d} pass={dir_c.get((d,'pass'),0):5d} uns={dir_c.get((d,'unsupported'),0)}")
print("\n=== distinct fail reasons (top 50) ===")
rc=collections.Counter()
for x in sel:
    if x['status']=='fail': rc[x.get('reason','')]+=1
for reason,n in rc.most_common(50):
    print(f"{n:5d}  {reason[:160]}")
