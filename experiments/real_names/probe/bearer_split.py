import json, subprocess, sys
EN = {"Q30","Q145","Q16","Q408","Q664","Q27"}
def q(s):
    out = subprocess.run(["curl","-s","-m","600","-G","https://qlever.dev/api/wikidata","--data-urlencode","query="+s,
        "-H","Accept: application/sparql-results+json"],capture_output=True,text=True,check=True).stdout
    return json.loads(out)["results"]["bindings"]
names = sys.argv[1:]
vals = " ".join(f'"{n}"@mul "{n}"@en' for n in names)
rows = q(f"""PREFIX wd: <http://www.wikidata.org/entity/> PREFIX wdt: <http://www.wikidata.org/prop/direct/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?nl ?c (COUNT(DISTINCT ?p) AS ?b) WHERE {{ VALUES ?nl {{ {vals} }} ?n rdfs:label ?nl ; wdt:P31/wdt:P279* wd:Q101352 .
 ?p wdt:P734 ?n . OPTIONAL {{ ?p wdt:P27 ?c }} }} GROUP BY ?nl ?c""")
agg = {}
for r in rows:
    n = r["nl"]["value"]; c = r.get("c", {}).get("value", "").rsplit("/",1)[-1]; b = int(r["b"]["value"])
    t = agg.setdefault(n, {})
    t[c] = t.get(c, 0) + b
for n in names:
    t = agg.get(n, {}); tot = sum(t.values()); en = sum(v for k, v in t.items() if k in EN)
    top = sorted(((v,k) for k,v in t.items() if k), reverse=True)[:3]
    print(f"{n:12} {tot:6}  English-speaking {en/max(tot,1):4.0%}  top {top}")
