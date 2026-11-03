import json, random, sys
random.seed(int(sys.argv[1])); N=int(sys.argv[2])
atoms=["a","A","1","9","0",".",",",":",";","-","--","/","$","%","@","#","&","'","’","\"","“","(",")","[","]","](/","/)","[a](/ə/)"," ","  ","\n","\n\n","\t","…","!","?","é","́","Б","α","東","😀","ー","。","，","http://","www.",".com","km","pm","th","st","Mr.","MR.","DR.","etc.","U.S.","NASA","they're","yeah","(s)","hundred"," billion","k","e","x","[pause:0.1s]","​"," ","ﬁ","ａ","Ⅻ","½","²","₹","€","+1 ","555","1,234","3.14","1.2.3","12:30"]
for k in range(N):
    m=random.randint(1,4)
    unit="".join(random.choice(atoms) for _ in range(random.randint(1,6)))
    sep=random.choice([""," ",". ",", ","\n"])
    u=unit+sep
    for t in (1250,5000):
        s=(u*(t//len(u)+1))[:t]
        print(json.dumps({"name":repr(u),"text":s}))
