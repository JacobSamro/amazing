#!/usr/bin/env python3
"""Plain native content aliases: preserve every path and byte, never link originals."""
import os
from pathlib import Path


def key(row):return row['sha256'],row['bytes'],row['mode'],row['elf']
def is_plain(row,packed,data):return row['label'] not in packed or (row['label'] in data and row['elf'])


def capacity(rows,directories,packed,data,deduplicate,budget,reserve,available):
    labels={};unique={};plain=0
    for row in rows:
        info=labels.setdefault(row['label'],{'files':0,'logical_bytes':0,'plain_bytes':0,'elf_files':0,'elf_bytes':0,'packed_data_bytes':0,'candidate_unique_plain_bytes':0});info['files']+=1;info['logical_bytes']+=row['bytes'];info['elf_files']+=int(row['elf']);info['elf_bytes']+=row['bytes'] if row['elf'] else 0
        if is_plain(row,packed,data):plain+=row['bytes'];info['plain_bytes']+=row['bytes'];unique.setdefault(key(row),row['bytes'])
        else:info['packed_data_bytes']+=row['bytes']
    for label,info in labels.items():info['candidate_unique_plain_bytes']=sum({key(r):r['bytes'] for r in rows if r['label']==label and is_plain(r,packed,data)}.values())
    overhead=4096*(len(rows)+len(directories))+65536;candidate=sum(unique.values())
    return {'labels':labels,'logical_bytes':sum(r['bytes'] for r in rows),'plain_bytes':plain,'candidate_unique_plain_bytes':candidate,'candidate_duplicate_bytes':plain-candidate,'metadata_allocation_bytes':overhead,'required_plain_and_metadata_bytes':(candidate if deduplicate else plain)+overhead,'required_without_dedup_bytes':plain+overhead,'dedup_requested':deduplicate,'copy_budget_bytes':budget,'reserve_bytes':reserve,'actual_available_bytes':available,'compressed_bytes':'not estimated; measured under bounded write guard before complete publication'}


def equal_bytes(left,right):
    with left.open('rb') as a,right.open('rb') as b:
        while True:
            x,y=a.read(1024*1024),b.read(1024*1024)
            if x!=y:return False
            if not x:return True


def verify_aliases(destination,rows,aliases):
    declared={(r['label'],r['path']):r for r in rows}
    seen=set()
    for alias in aliases:
        left=(alias['label'],alias['path']);right=(alias['owner_label'],alias['owner_path'])
        if left==right or left in seen or left not in declared or right not in declared:raise ValueError('invalid captured content alias inventory')
        if key(declared[left])!=key(declared[right]):raise ValueError('content alias metadata mismatch')
        a=destination/'payload'/left[0]/left[1];b=destination/'payload'/right[0]/right[1]
        if a.is_symlink() or b.is_symlink() or not a.is_file() or not b.is_file():raise ValueError('plain content alias path missing')
        ai,bi=a.stat(),b.stat()
        if (ai.st_dev,ai.st_ino)!=(bi.st_dev,bi.st_ino):raise ValueError('captured content aliases are not actual native links')
        seen.add(left)
    return {'verified_aliases':len(seen),'alias_storage':'hardlinks ONLY between byte-compared new captured files; no original inode reused; every logical path plain/readable'}
