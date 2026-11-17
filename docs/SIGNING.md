# 代码签名（Authenticode）

未签名的 exe 在 Windows 上首次运行会弹「Windows 已保护你的电脑 / 未知发布者」。签名后显示发布者名称，SmartScreen 信誉也会随下载量累积提升。

## 一、准备证书
- 购买 **OV/EV Code Signing Certificate**（DigiCert / Sectigo / SSL.com 等），导出为带私钥的 `.pfx`。
- 记下 `.pfx` 的密码。

## 二、配置 GitHub Secrets
在仓库 `Settings → Secrets and variables → Actions` 添加：

| Secret | 内容 |
|---|---|
| `WINDOWS_PFX_BASE64` | `.pfx` 文件的 Base64（见下） |
| `WINDOWS_PFX_PASSWORD` | `.pfx` 密码 |

生成 Base64（PowerShell）：
```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("D:\path\cert.pfx")) | Set-Clipboard
```

配置后，`.github/workflows/release.yml` 里的两个「Code sign (optional)」步骤会自动：
1. 用 `signtool` 对 `target\release\myisle.exe` 签名（时间戳 `timestamp.digicert.com`）；
2. 对 `dist\MyIsle-Setup-<ver>.exe` 再签一次。

没有配置这两个 Secret 时，步骤自动跳过，其余发布流程不受影响。

## 三、本地手签（可选）
```powershell
$signtool = (Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter signtool.exe |
  Where-Object { $_.FullName -match 'x64' } | Select-Object -First 1).FullName
& $signtool sign /f cert.pfx /p <密码> /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 .\MyIsle.exe
```

## 四、注意
- EV 证书通常要求硬件令牌（USB），CI 需用云签名服务（如 Azure Trusted Signing / DigiCert KeyLocker），
  那种情况把 workflow 的签名步骤换成对应 CLI 即可。
- 证书有效期一般 1 年，到期前记得续期并更新 Secret。
