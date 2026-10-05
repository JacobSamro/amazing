#!/usr/bin/env python3
"""Explicit baseline compatibility; classifications and comparison algorithm unchanged."""
import hashlib,json,pathlib
LEGACY_RESOURCE='e4f678899d25b7bcba2f2995e1c10445a2ff1b55054a5c8d857b44a1c1302404'
LEGACY_COMPARATORS={'97bd438374544f3861ad79fafbc587aa0dbd8b7506b789cfbd5cc92e9ba8a8db','b9bc57831fd2045d5d3351fed890ad35c702e5c3a429757db9d7df97ce6fd7bf'}
LEGACY_EXECUTORS={'4310869c9a008b546b0bf5e3ab24520f90c00beec5c047937c57279b7ca0fb2d','3b44a00fd8824b3c0ec4a30c595e2d6e9916d5b0963401c93f60b3efff01e57d','89305027860b23b3ce4e702e7063146c0e813f7efaef5b4e57a7260e2657ba43'}
REVIEWED_CURRENT_HELPERS={'image-resource-corpus.py': 'de54c2e13e991f1a1f1c095ff8dff317184d8e9afc5250ed5f7078a66b144456', 'wpt_render_compare.py': '219764b940f2b41da580f1eb73634088b96203b954f19fd72bf29dfba4f14b23', 'image-presentation-gate.py': 'ae3091556bd0993ea9ce864cb9eeed0d82dc7490a6ddd3699a20f2c14df62ba2'}
def compatible_helper(name, baseline_sha, current_sha):
 if baseline_sha==current_sha:return True
 allowed={'image-resource-corpus.py':{LEGACY_RESOURCE},'wpt_render_compare.py':LEGACY_COMPARATORS,'image-presentation-gate.py':LEGACY_EXECUTORS}
 return current_sha==REVIEWED_CURRENT_HELPERS.get(name) and baseline_sha in allowed.get(name,set())
def validate_resource_metadata(baseline,current,baseline_role,baseline_source,current_ids):
 if baseline_role not in ('before','after') or baseline.get('corpus_role')!=baseline_role or current.get('corpus_role')!='after':raise ValueError('resource comparison role mismatch; baseline must retain original role')
 if baseline.get('source_commit')!=baseline_source or not baseline_source:raise ValueError('resource baseline source mismatch')
 for key in ('revision','rc21_entries','manifest_sha256','settings','selection_sha256','profile'):
  if baseline[key]!=current[key]:raise ValueError('resource metadata differs: '+key)
 if set(baseline['tests'])!=set(current_ids):raise ValueError('resource case IDs changed')
 for name,key in [('image-resource-corpus.py','adapter_sha256'),('wpt_render_compare.py','comparator_sha256')]:
  if not compatible_helper(name,baseline[key],current[key]):raise ValueError('resource classifier source differs: '+name)
def compare_resource(baseline,current,baseline_role,baseline_source):
 validate_resource_metadata(baseline,current,baseline_role,baseline_source,current['tests'])
 old={k for k,v in baseline['tests'].items() if v=='pass'};new={k for k,v in current['tests'].items() if v=='pass'}
 return {'lost':sorted(old-new),'gained':sorted(new-old)}
