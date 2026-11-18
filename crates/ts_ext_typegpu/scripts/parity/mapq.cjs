// Map quality: for every identifier token in the output, find the segment that starts exactly at it.
const fs=require("fs");
const NM=process.env.PARITY_NODE_MODULES + "/";
const {decode}=require(NM+"@jridgewell/sourcemap-codec");
function* jsTokens(src){const re=/("(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|`(?:\\.|[^`\\])*`|\/\/[^\n]*|\/\*[\s\S]*?\*\/|[A-Za-z_$][\w$]*|\s+|[\s\S])/g;let m;while((m=re.exec(src))){const v=m[0];yield {value:v,type:/^[A-Za-z_$]/.test(v)?"IdentifierName":"Other"};}}
const GENERATED=/^(__tsover_\w+|globalThis|__TYPEGPU_\w+|WeakMap|\$|undefined)$/;
function quality(source,output,map){
  const lines=decode(map.mappings); const srcLines=source.split("\n");
  const outLines=output.split("\n");
  const at=new Map(); lines.forEach((segs,l)=>segs.forEach(s=>{if(s.length>=4) at.set(l+":"+s[0],s);}));
  const srcIdents=new Set([...source.matchAll(/[A-Za-z_$][\w$]*/g)].map(m=>m[0]));
  let exact=0,wrong=0,unmapped=0; const bad=[];
  // Identifier tokens only (js-tokens skips strings, comments, regexes).
  let l=0,col=0;
  for(const t of jsTokens(output,{jsx:true})){
    if(t.type==="IdentifierName"&&!GENERATED.test(t.value)){
      const tok=t.value; const s=at.get(l+":"+col);
      if(!s){ if(srcIdents.has(tok)){unmapped++; if(bad.length<8) bad.push(`unmapped ${tok}@${l+1}:${col}`);} }
      else { const orig=(srcLines[s[2]]??"").slice(s[3]); if(orig.startsWith(tok)) exact++; else if(srcIdents.has(tok)){wrong++; if(bad.length<8) bad.push(`wrong ${tok}@${l+1}:${col} -> ${s[2]+1}:${s[3]} "${orig.slice(0,12)}"`);} }
    }
    const parts=t.value.split("\n"); if(parts.length>1){l+=parts.length-1; col=parts[parts.length-1].length;} else col+=t.value.length;
  }
  return {exact,wrong,unmapped,bad};
}
module.exports={quality};
if(require.main===module){
  const {Transformer}=require(require("path").join(__dirname, "../../../ts_ext_syntax/js/index.cjs"));
  const t=new Transformer({tsconfig:false});
  const files=process.argv.slice(2);
  let tot={exact:0,wrong:0,unmapped:0},badFiles=0;
  for(const f of files){const src=fs.readFileSync(f,"utf8");const r=t.transform(src,f);const q=quality(src,r.code,r.map);
    for(const k in tot)tot[k]+=q[k]; if(q.wrong||q.unmapped){badFiles++; if(process.env.V) console.log(f.split("/").pop(),q.wrong,q.unmapped,q.bad.join(" | "));}}
  console.log(JSON.stringify(tot),"files",files.length,"imperfect",badFiles);
}
