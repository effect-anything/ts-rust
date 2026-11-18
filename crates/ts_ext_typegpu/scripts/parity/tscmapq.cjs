const fs=require("fs");const {quality}=require("./mapq.cjs");
for(const dir of process.argv.slice(2)){let tot={exact:0,wrong:0,unmapped:0},cleared=0;
for(const f of fs.readdirSync(dir+"/src")){const src=fs.readFileSync(dir+"/src/"+f,"utf8");const js=f.replace(/\.tsx?$/,".js");
const out=fs.readFileSync(dir+"/out/"+js,"utf8");const map=JSON.parse(fs.readFileSync(dir+"/out/"+js+".map","utf8"));if(!map.mappings)cleared++;
const q=quality(src,out,map);for(const k in tot)tot[k]+=q[k];}
console.log(dir,JSON.stringify(tot),"maps cleared",cleared);}
