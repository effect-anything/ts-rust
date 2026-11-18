const fs=require("fs"),path=require("path");
const oxc=require(require("path").join(process.env.PARITY_NODE_MODULES, "oxc-transform"));
const {Transformer}=require(require("path").join(__dirname, "../../../ts_ext_syntax/js/index.cjs"));
const t=new Transformer({tsconfig:false});
const strip=(code,f)=>{const r=oxc.transformSync(f,code,{lang:f.endsWith("x")?"tsx":"ts",sourcemap:false});if(r.errors.length)throw new Error(f+": "+r.errors.map(e=>e.message).join(";"));return r.code;};
const norm=(code)=>strip(code.replace(/\/\/# sourceMappingURL=.*/,""),"n.ts").replace(/\n\s*\n/g,"\n").replace(/^"use strict";\n/,"");
let same=0,diff=[];
for(const f of fs.readdirSync("proj/src")){
  const src=fs.readFileSync("proj/src/"+f,"utf8");
  const tsc=fs.readFileSync("proj/out/"+f.replace(/\.tsx?$/,".js"),"utf8");
  const r=t.transform(src,"/p/"+f);
  let ours; try{ours=strip(r.code,f);}catch(e){diff.push([f,"STRIP "+e.message]);continue;}
  const A=norm(tsc),B=norm(ours);
  if(A===B) same++; else {diff.push([f]); fs.mkdirSync("d",{recursive:true});fs.writeFileSync("d/"+f+".tsc.js",A);fs.writeFileSync("d/"+f+".ours.js",B);}
}
console.log("same",same,"diff",diff.length); for(const d of diff) console.log(d.join(" "));
