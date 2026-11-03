# From the PR #19 review (copied from its scratch directory): test texts for reference.py.
import json, random, sys
random.seed(7)
nums = lambda: random.choice([str(random.randint(0,10**random.randint(1,14))), f"{random.randint(0,999)},{random.randint(0,999):03d}", f"-{random.randint(0,99)}", f"−{random.randint(0,99)}.{random.randint(0,99)}", f"{random.random()*100:.3f}", "".join(chr(0xff10+int(c)) for c in str(random.randint(0,10**8))), f"{random.randint(1,9)}億{random.randint(1,9999)}万", f"二〇{random.randint(10,29)}年", f"{random.randint(1,12)}月{random.randint(1,31)}日", f"{random.randint(1,99)}{random.choice('人本匹杯個回階時分秒歳枚台冊頭羽件軒足着点')}", f"第{random.randint(1,99)}", f"{random.randint(1,9)}〜{random.randint(10,99)}", f"{random.randint(1,9)}～{random.randint(10,99)}%", f"{random.randint(1,99)}％", f"0{random.randint(0,99)}", "１.５", f"{random.randint(1,9)}.{random.randint(0,9)}.{random.randint(0,9)}"])
frag = ["吾輩《わがはい》は","｜猫《ねこ》","［＃「猫」に傍点］","※［＃「てへん＋劣」、第3水準1-84-77］","〔Paris〕","ヴァイオリン","ヴィヴァルディ","ヴェルサイユ","ゔぁ","ッー","ーッ","っっ","ちょっとー","すっごーい","ぇー","ぃぃ","ヵ月","ヶ所","ゝ","ゞ","々","〃","ｳﾞｧ","ﾍﾟﾝ","ｯｰ","が","ｶ゙","゚","ＡＢＣ","ｘｙｚ","Ｗｉ－Ｆｉ","、","。","「","」","（","）","！","？","…","・","ー","　"," ","東京都","大阪府","行きました","ありがとうございました","と言った","のです","ではない","コンピューター","データベース","ファイル","ウィンドウ","ティッシュ","ドゥ","チェッ"]
out=open(sys.argv[1],"w")
for i in range(6000):
    k=random.randint(1,120 if i%10==0 else 20)
    s="".join(nums() if random.random()<0.35 else random.choice(frag) for _ in range(k))
    out.write(json.dumps({"text":s},ensure_ascii=False)+"\n")
