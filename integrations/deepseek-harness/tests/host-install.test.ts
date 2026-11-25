import {Context} from '@deepseek-ai/cordis';
import SessionStore from '@deepseek-ai/dsh-session';
import SessionProjectionRegistry from '@deepseek-ai/dsh-session-projection';
import{beforeAll,it,expect}from'vitest';
import{execFileSync}from'node:child_process';import{mkdtemp,readFile,access,rm}from'node:fs/promises';import{tmpdir}from'node:os';import{join}from'node:path';import{fileURLToPath,pathToFileURL}from'node:url';import vm from'node:vm';
import{createInstallPackage}from'../create-install.mjs';
function resolvePnpmInvocation(args:string[]):{command:string;args:string[]}{
 if(process.platform!=='win32')return {command:'pnpm',args};
 const npmExecPath=process.env.npm_execpath;
 if(npmExecPath){
  const lower=npmExecPath.toLowerCase();
  if(lower.endsWith('.js')||lower.endsWith('.cjs')||lower.endsWith('.mjs'))return {command:process.execPath,args:[npmExecPath,...args]};
  if(lower.endsWith('.exe')||lower.endsWith('.com'))return {command:npmExecPath,args};
 }
 return {command:process.env.ComSpec||'cmd.exe',args:['/d','/s','/c','pnpm',...args]};
}
beforeAll(()=>{const invocation=resolvePnpmInvocation(['--filter','@codex-pet/deepseek-integration','build']);execFileSync(invocation.command,invocation.args,{cwd:fileURLToPath(new URL('../../../',import.meta.url)),stdio:'pipe'});},120000);
it('离线生成可解析bundle：中文空格新路径、精确构建文件、原安装ID升级保留',async()=>{
 const temporary=await mkdtemp(join(tmpdir(),'安装 包-'));const output=join(temporary,'中文 空格');const id='11111111-1111-4111-8111-111111111111';
 try{
  const result=await createInstallPackage({outputDir:output,bridgePath:join(temporary,'bridge exe'),probeDir:join(temporary,'probe'),installationId:id});
  expect(await access(join(output,'package.json')).then(()=>true,()=>false)).toBe(true);
  const manifest=JSON.parse(await readFile(join(output,'package.json'),'utf8'));const patch=JSON.parse(await readFile(join(output,manifest.dsh.bundle.patch),'utf8'));
  expect(patch[0].insert[0].config.installationId).toBe(id);expect(result.installationId).toBe(id);
  expect(manifest.exports['.']).toBe('./dist/index.js');
  expect(manifest.engines.dsh).toBe('>=0.2.2-rc.1 <0.3.0-0');
  expect(manifest.peerDependencies['@deepseek-ai/cordis']).toBe('^4.1.0 || ^4.2.0-alpha.1');
  expect(manifest.peerDependencies['@deepseek-ai/dsh-session']).toBe('^0.2.2-rc.1 || ^0.2.3-alpha.1');
  expect(manifest.dependencies).toBeUndefined();
  expect(Object.values({...manifest.dependencies,...manifest.peerDependencies}).every(value=>typeof value==='string'&&!/^(?:file:|link:|workspace:|\/)/.test(value))).toBe(true);
  expect(await readFile(join(output,'dist/index.js'),'utf8')).toBe(await readFile(new URL('../dist/index.js',import.meta.url),'utf8'));
  const isolatedHost=await import(pathToFileURL(join(output,'dist/index.js')).href);
  expect(typeof isolatedHost.apply).toBe('function');
  expect(isolatedHost.inject).toEqual(['sessions','sessionProjections']);
  expect(isolatedHost.Config.parse(patch[0].insert[0].config)).toEqual(patch[0].insert[0].config);
  expect(()=>isolatedHost.Config.parse({bridgePath:'relative/pet-bridge',installationId:'not-a-uuid'})).toThrow();
  const ctx=new Context();
  await ctx.plugin(SessionStore);
  await ctx.plugin(SessionProjectionRegistry);
  await ctx.plugin(isolatedHost,patch[0].insert[0].config);
  await ctx.fiber.dispose();
  const isolatedOpenSession=await import(pathToFileURL(join(output,'dist/open-session.js')).href);
  expect(typeof isolatedOpenSession.openHarnessSession).toBe('function');
  let factory: {id:string;factory:(require:(name:string)=>never)=>{apply:unknown}}|undefined;
  vm.runInNewContext(await readFile(join(output,'dist/client.js'),'utf8'),{window:{__ModuleLoader__:{load:(row:typeof factory)=>{factory=row;}}}});
  expect(factory?.id).toBe('@codex-pet/deepseek-integration');expect(typeof factory?.factory(()=>{throw Error('未声明外部依赖');}).apply).toBe('function');
  await expect(createInstallPackage({outputDir:output,bridgePath:join(temporary,'other')})).rejects.toThrow();
  expect(JSON.parse(await readFile(join(output,'cordis.patch.json'),'utf8'))[0].insert[0].config.installationId).toBe(id);
  const upgraded=await createInstallPackage({outputDir:join(temporary,'升级 包'),bridgePath:join(temporary,'bridge'),installationId:result.installationId});expect(upgraded.installationId).toBe(result.installationId);
  const fresh=await createInstallPackage({outputDir:join(temporary,'首次'),bridgePath:join(temporary,'bridge')});expect(fresh.installationId).toMatch(/^[0-9a-f-]{36}$/);
 }finally{await rm(temporary,{recursive:true,force:true});}
});
