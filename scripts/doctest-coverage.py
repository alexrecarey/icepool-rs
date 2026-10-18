#!/usr/bin/env python3
"""Which public items of icepool-core have no example (a ``` block) in their docs.

Reads rustdoc's JSON (nightly only):

    RUSTDOCFLAGS="-Z unstable-options --output-format json" cargo +nightly doc -p icepool-core --no-deps
    python3 scripts/doctest-coverage.py $CARGO_TARGET_DIR/doc/icepool_core.json evaluator,expression,value [-v] [-m]

The second argument names the root modules to walk (re-exports followed to their definitions, each item counted
once). Prints the number of top-level items and of inherent and trait methods without an example; `-v` lists the
items, `-m` the methods.
"""
import json,sys
j=json.load(open(sys.argv[1])); idx=j['index']
def kind(it): return next(iter(it['inner']))
def has_ex(it): return '```' in (it.get('docs') or '')
seen=set(); top=[]; methods=[]
def visit(id_, path):
    it=idx.get(str(id_))
    if it is None: return
    k=kind(it)
    if k=='use':
        u=it['inner']['use']
        if u['id'] is None: return
        t=idx.get(str(u['id']))
        if t is None: return
        if u['is_glob']:
            visit(u['id'], path); return
        visit_item(u['id'], path+'::'+u['name'], it)
    else:
        visit_item(id_, path+'::'+(it['name'] or ''), None)
def visit_item(id_, p, use):
    it=idx[str(id_)]; k=kind(it)
    if k=='module':
        for c in it['inner']['module']['items']: visit(c, p)
        return
    if id_ in seen: return
    seen.add(id_)
    top.append((p,k,has_ex(it),(it.get('span') or {}).get('filename','?'),(it.get('span') or {}).get('begin',[0])[0]))
    impls=[]
    if k in('struct','enum','union'):
        impls=it['inner'][k]['impls']
    if k=='trait':
        for m in it['inner']['trait']['items']:
            mi=idx[str(m)]; methods.append((p+'::'+mi['name'],has_ex(mi)))
    for im in impls:
        ii=idx[str(im)]['inner']['impl']
        if ii['trait'] is not None or ii.get('is_synthetic') or ii.get('blanket_impl'): continue
        for m in ii['items']:
            mi=idx[str(m)]
            if mi.get('visibility')=='public':
                methods.append((p+'::'+mi['name'],has_ex(mi)))
root=idx[str(j['root'])]
for c in root['inner']['module']['items']:
    it=idx[str(c)]
    n=it['name']
    if kind(it)=='module' and n in sys.argv[2].split(','):
        visit_item(c, n, None)
miss=[t for t in top if not t[2]]; mm=[m for m in methods if not m[1]]
print('top',len(top),'missing',len(miss)); print('methods',len(methods),'missing',len(mm))
if '-v' in sys.argv:
    for t in miss: print('T',t[0],t[1],'%s:%s'%(t[3],t[4]))
if '-m' in sys.argv:
    for m in mm: print('M',m[0])
