# From the PR #19 review (copied from its scratch directory): test texts for reference.py.
import json, random, sys
random.seed(int(sys.argv[1])); n=int(sys.argv[2])
pool = list("あいうえおかがきぎくぐっゃゅょぁぃぅぇぉんーゝゞヽヾ々〃アイウエオカガキッャュョァィゥェォンヴヵヶヷヸヹヺㇰㇱㇷㇿｱｲｳｶﾞｯｰｧﾟﾞ東京大阪日本語漢字一二三四五六七八九十百千万億兆〇零円年月日人本匹杯個回階時分秒歳第、。・「」『』（）【】！？…〜～ 　0123456789０１２３４５６７８９.,，．-−+%％/:abcXYZＡＢｃ") + ["゙","゚","゛","゜","😀","𠮷","٣","০","❶","①","Ⅳ","½","²","\t","\n","《","》","｜","［＃","］","​","﻿"]
words = ["東京","ヴァイオリン","コンピューター","ありがとう","すごーい","やったー","ちょっと","きゃー","パーティー","ディズニーランド","１,０００","3.14","-5","二〇二三年","12,345","100万","1億2345万","5人","3本","10個","4匹","6杯","8分","１０:３０","吾輩《わがはい》","｜東京《とうきょう》","Ｗｉ－Ｆｉ","ＮＨＫ","iPhone","ﾃﾞｨｽﾞﾆｰ"]
out=open(sys.argv[3],"w")
for _ in range(n):
    k=random.randint(1,25); s=""
    for _ in range(k):
        s += random.choice(words) if random.random()<0.3 else random.choice(pool)
    out.write(json.dumps({"text":s},ensure_ascii=False)+"\n")
