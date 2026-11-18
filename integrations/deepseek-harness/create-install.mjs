import {randomUUID} from 'node:crypto';
import {readFile,writeFile,mkdir,cp,rm} from 'node:fs/promises';
import {isAbsolute,join,resolve} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';

/** 生成独立可安装bundle；只创建指定新目录，UUID保存在配置中，不写宿主profile。 */
export async function createInstallPackage(options){
  if(typeof options.outputDir!=='string'||!isAbsolute(options.outputDir))throw new Error('输出必须是不存在的绝对目录');
  const packageRoot=fileURLToPath(new URL('./',import.meta.url));
  const {Config}=await import('./dist/index.js');
  const config=Config.parse({bridgePath:options.bridgePath,installationId:options.installationId??randomUUID(),...(options.probeDir===undefined?{}:{probeDir:options.probeDir})});
  const manifest=JSON.parse(await readFile(join(packageRoot,'package.json'),'utf8'));
  delete manifest.devDependencies;delete manifest.scripts;
  manifest.dsh={...manifest.dsh,bundle:{patch:'./cordis.patch.json'}};
  manifest.files=['dist','cordis.patch.json','README.md','compatibility.json'];
  await mkdir(options.outputDir,{mode:0o700});
  try{
    await cp(join(packageRoot,'dist'),join(options.outputDir,'dist'),{recursive:true,errorOnExist:true,force:false});
    for(const name of ['README.md','compatibility.json'])await cp(join(packageRoot,name),join(options.outputDir,name),{errorOnExist:true,force:false});
    await writeFile(join(options.outputDir,'package.json'),JSON.stringify(manifest,null,2)+'\n',{flag:'wx',mode:0o600});
    await writeFile(join(options.outputDir,'cordis.patch.json'),JSON.stringify([{insert:[{id:'desktop-pet-notify',name:manifest.name,config}]}],null,2)+'\n',{flag:'wx',mode:0o600});
  }catch(error){await rm(options.outputDir,{recursive:true,force:true});throw error;}
  return {outputDir:options.outputDir,installationId:config.installationId};
}

if(process.argv[1]!==undefined&&pathToFileURL(resolve(process.argv[1])).href===import.meta.url){
  try{
    const keys={'--output':'outputDir','--bridge':'bridgePath','--probe-dir':'probeDir','--installation-id':'installationId'};
    const options={};const args=process.argv.slice(2);
    for(let i=0;i<args.length;i+=2){const key=keys[args[i]];if(key===undefined||options[key]!==undefined||args[i+1]===undefined)throw new Error('参数缺失、重复或无效');options[key]=args[i+1];}
    const result=await createInstallPackage(options);
    console.log('已生成可审阅的离线配置包：'+result.outputDir+'；安装ID：'+result.installationId);
  }catch(_error){console.error('生成失败：请先构建本包，并提供有效bridge绝对路径、UUID及不存在的输出绝对目录');process.exitCode=1;}
}
