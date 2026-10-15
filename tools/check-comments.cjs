'use strict'
// 注释语言检查只扫描自有源码；依赖、许可原文和工具生成锁文件不改写。
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict')
const acorn=require('./ui-tests/node_modules/acorn')
const root=path.resolve(__dirname,'..'),issues=[],excluded=new Set(['node_modules','target','tmp','dist','.git'])
function walk(directory){return fs.readdirSync(directory,{withFileTypes:true}).flatMap(entry=>{if(excluded.has(entry.name))return [];const file=path.join(directory,entry.name);return entry.isDirectory()?walk(file):[file]})}
function rust(text){
 let i=0,comments=[]
 while(i<text.length){
  const raw=/^(?:b)?r(#{0,32})"/.exec(text.slice(i))
  if(raw){const end=text.indexOf('"'+raw[1],i+raw[0].length);assert.ok(end>=0);i=end+1+raw[1].length;continue}
  const char=/^(?:b)?'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\r\n])'/u.exec(text.slice(i))
  if(char){i+=char[0].length;continue}
  if(text[i]==='"'){i++;while(i<text.length){if(text[i]==='\\'){i+=2;continue}if(text[i++]==='"')break}continue}
  if(text.startsWith('//',i)){const end=text.indexOf('\n',i);const stop=end<0?text.length:end;comments.push(text.slice(i+2,stop));i=stop;continue}
  if(text.startsWith('/*',i)){const start=i;i+=2;let depth=1;while(depth&&i<text.length){if(text.startsWith('/*',i)){depth++;i+=2}else if(text.startsWith('*/',i)){depth--;i+=2}else i++}assert.equal(depth,0);comments.push(text.slice(start+2,i-2));continue}
  i++
 }return comments
}
let comments=0,files=0
for(const file of walk(root)){
 const extension=path.extname(file),text=fs.readFileSync(file,'utf8');let values=[]
 if(['.js','.cjs'].includes(extension))acorn.parse(text,{ecmaVersion:'latest',allowReturnOutsideFunction:true,onComment:(_block,value)=>values.push(value)})
 else if(extension==='.rs')values=rust(text)
 else if(extension==='.css')values=[...text.matchAll(/\/\*([\s\S]*?)\*\//g)].map(x=>x[1])
 else if(extension==='.html')values=[...text.matchAll(/<!--([\s\S]*?)-->/g)].map(x=>x[1])
 else continue
 files++;comments+=values.length
 for(const value of values)if(value.trim()&&!/[\u3400-\u9fff]/.test(value))issues.push(path.relative(root,file))
}
assert.deepEqual(issues,[],'业务注释必须有中文说明，协议名称和来源URL可保留')
console.log(JSON.stringify({files,comments,scope:'仅自有源码注释，非文档语言或生成文件'}))

