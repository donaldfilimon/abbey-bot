import re, json, sys
lit = re.compile(r'"((?:[^"\\]|\\.)*)"')
def unescape(s):
    out=[];i=0
    while i<len(s):
        c=s[i]
        if c=='\\':
            n=s[i+1]
            if n=='u':
                j=s.index('}',i); out.append(chr(int(s[i+3:j],16))); i=j+1; continue
            out.append({'n':'\n','t':'\t','r':'\r','\\':'\\','"':'"',"'":"'",'0':'\0'}.get(n,n)); i+=2; continue
        out.append(c); i+=1
    return ''.join(out)
seen=[]
for path in sys.argv[1:]:
    src=open(path).read()
    t=src.find('mod tests') if 'mod tests' in src else 0
    for m in lit.finditer(src[t:]):
        v=unescape(m.group(1))
        if '{' in v and '}' in v: continue
        if v not in seen: seen.append(v)
extra=["", "   ", "Abbey", "abbey", "ABBEY: hi", "@aviva hi", "@abi", "abi,", "Abi:x", "aviva\u00a0hi", "Aviva\u000bnow", "Aviva.x", "explain the deploy policy risk", "HELP ME NOW PLEASE", "why is this so confusing?!", "restart it now", "the server is down right now!!", "please fix the build", "could you run the tests now", "I'm losing my mind with this bug", "no idea what is happening", "caf\u00e9 na\u00efve \u00c9COLE", "\u0130stanbul", "\u03a3\u039f\u03a6\u0399\u0391 wisdom", "\U0001F642 run", "run\u2019s", "RUNNING fast", "compare, analyze; explain!", "policy? governance: routing.", "fixing", "fix-it", "deploy\tdeploy\ndeploy", "\ufeffhelp", "Ünïcödé HELP ÖVERLOAD"]
for e in extra:
    if e not in seen: seen.append(e)
json.dump(seen, open(sys.argv[0].replace('extract_inputs.py','golden-inputs.json'),'w'), ensure_ascii=False, indent=0)
print(len(seen))
