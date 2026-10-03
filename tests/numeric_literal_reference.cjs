// Original reference probes for NumericLiteral grammar and value boundaries.
const expressions=['077','0010','000','08','009','078','08.1_2','09e1_0','077.toString()','08..toString()','00.1','077.1','00e1','077e1','00n','077n','08n','0_0','00_1','08_1','0_1n','0x_1','0o8','0b2','0x1_n','1__0','1_.0','1._0','1e_1','1e+_1','1n0','0o0007','0x'+ 'f'.repeat(40),'0x1'+'0'.repeat(300),'0x20000000000001','0x20000000000003'];
const cases={};
for(const expression of expressions){
 try{const value=Function('return ('+expression+')')();cases[expression]={ok:true,type:typeof value,value:String(value)};}
 catch(error){cases[expression]={ok:false,error:error.name};}
}
for(const source of ["'use strict';0o007","'use strict';077","class C{0o007(){}}","class C{077(){}}","class C{08=1}","({999999999999999999999n:1})"]){
 try{Function(source);cases[source]={parse:true};}catch(error){cases[source]={parse:false,error:error.name};}
}
console.log(JSON.stringify({node:process.version,v8:process.versions.v8,cases},null,2));
