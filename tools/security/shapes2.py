import json, sys
T=20000
def fit(u,t):
    out=[];n=0;i=0
    while True:
        s=u(i)
        if n+len(s)>t: break
        out.append(s);n+=len(s);i+=1
    return "".join(out)
units={}
for L in (5,10,15,20,25,30,36,40,60,100,200,400):
    units[f"digits{L}"]=(lambda L: lambda i: "9"*L+" ")(L)
    units[f"neg digits{L}"]=(lambda L: lambda i: "-"+"7"*L+" ")(L)
    units[f"decimal{L}"]=(lambda L: lambda i: "0."+"7"*L+" ")(L)
    units[f"money{L}"]=(lambda L: lambda i: "$"+"7"*L+".99 ")(L)
    units[f"ordinal{L}"]=(lambda L: lambda i: "7"*L+"th ")(L)
    units[f"frac{L}"]=(lambda L: lambda i: "7"*L+"/"+"3"*L+" ")(L)
    units[f"time{L}"]=(lambda L: lambda i: "7"*L+":30 ")(L)
units["thousands groups"]=lambda i: "1"+",234"*20+" "
units["thousands groups long"]=lambda i: "1"+",234"*200+" "
units["money billion big"]=lambda i: "$"+"9"*30+" quadrillion "
units["year-ish"]=lambda i: "1999s "
units["digit S"]=lambda i: "1990S "
units["range dash"]=lambda i: "1-2-"
units["range dash big"]=lambda i: "9"*30+"-"+"9"*30+" "
units["versions big"]=lambda i: ".".join(["99999"]*10)+" "
units["phone big"]=lambda i: "555-123-4567-"
units["percent big"]=lambda i: "9"*30+"% "
units["units big"]=lambda i: "9"*30+" km "
units["number multipliers"]=lambda i: "5"+" hundred thousand million billion"*3+" "
units["decimal money"]=lambda i: "$0."+"9"*30+" "
units["sci"]=lambda i: "1e308 "
units["mixed num words"]=lambda i: "1st 2nd 3rd 4th 1/2 3.14 $5 10% 12:30 1999 1,000 "
units["dots digits big"]=lambda i: ("9"*20+".")
for k,u in units.items():
    for t in (T//4, T):
        print(json.dumps({"name":k,"text":fit(u,t)}))
