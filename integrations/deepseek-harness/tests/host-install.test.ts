import{beforeAll,it,expect}from'vitest';
import{execFileSync}from'node:child_process';import{mkdtemp,readFile,access,rm}from'node:fs/promises';import{tmpdir}from'node:os';import{join}from'node:path';import{fileURLToPath}from'node:url';import vm from'node:vm';
import{createInstallPackage}from'../create-install.mjs';
beforeAll(()=>{execFileSync('pnpm',['--filter','@codex-pet/deepseek-integration','build'],{cwd:fileURLToPath(new URL('../../../',import.meta.url)),stdio:'pipe'});},120000);
it('离线生成可解析bundle：中文空格新路径、精确构建文件、原安装ID升级保留',async()=>{
 const temporary=await mkdtemp(join(tmpdir(),'安装 包-'));const output=join(temporary,'中文 空格');const id='11111111-1111-4111-8111-111111111111';
 try{
  const result=await createInstallPackage({outputDir:output,bridgePath:join(temporary,'bridge exe'),probeDir:join(temporary,'probe'),installationId:id});
  expect(await access(join(output,'package.json')).then(()=>true,()=>false)).toBe(true);
  const manifest=JSON.parse(await readFile(join(output,'package.json'),'utf8'));const patch=JSON.parse(await readFile(join(output,manifest.dsh.bundle.patch),'utf8'));
  expect(patch[0].insert[0].config.installationId).toBe(id);expect(result.installationId).toBe(id);
  expect(manifest.exports['.']).toBe('./dist/index.js');
  expect(Object.values({...manifest.dependencies,...manifest.peerDependencies}).every(value=>typeof value==='string'&&!/^(?:file:|link:|workspace:|\/)/.test(value))).toBe(true);
  expect(await readFile(join(output,'dist/index.js'),'utf8')).toBe(await readFile(new URL('../dist/index.js',import.meta.url),'utf8'));
  let factory: {id:string;factory:(require:(name:string)=>never)=>{apply:unknown}}|undefined;
  vm.runInNewContext(await readFile(join(output,'dist/client.js'),'utf8'),{window:{__ModuleLoader__:{load:(row:typeof factory)=>{factory=row;}}}});
  expect(factory?.id).toBe('@codex-pet/deepseek-integration');expect(typeof factory?.factory(()=>{throw Error('未声明外部依赖');}).apply).toBe('function');
  await expect(createInstallPackage({outputDir:output,bridgePath:join(temporary,'other')})).rejects.toThrow();
  expect(JSON.parse(await readFile(join(output,'cordis.patch.json'),'utf8'))[0].insert[0].config.installationId).toBe(id);
  const upgraded=await createInstallPackage({outputDir:join(temporary,'升级 包'),bridgePath:join(temporary,'bridge'),installationId:result.installationId});expect(upgraded.installationId).toBe(result.installationId);
  const fresh=await createInstallPackage({outputDir:join(temporary,'首次'),bridgePath:join(temporary,'bridge')});expect(fresh.installationId).toMatch(/^[0-9a-f-]{36}$/);
 }finally{await rm(temporary,{recursive:true,force:true});}
});
