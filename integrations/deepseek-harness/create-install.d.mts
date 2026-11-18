export interface InstallPackageOptions { outputDir:string; bridgePath:string; probeDir?:string; installationId?:string }
export function createInstallPackage(options:InstallPackageOptions):Promise<{outputDir:string;installationId:string}>;
