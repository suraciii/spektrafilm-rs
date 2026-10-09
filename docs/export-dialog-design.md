# GUI 照片导出产品设计

2026-10-09 · 设计提案，尚未实现到原生 GUI。配套原型：`export-dialog-prototype.html`。原型中的文件、任务状态均为交互演示，不执行照片编码。

## 目标与方案

让用户在一次导出中明确选择格式、JPEG 质量、色彩细节、位深和保存位置；开始前能看见真实输出尺寸。默认全尺寸重新渲染，不复用可能过时或缩小的预览。

选择应用内 egui「导出照片」弹窗，原生文件选择器仅负责路径。另一方案是在原生保存框增加所有选项，但跨平台扩展能力不一致，格式联动和校验也难统一，因此不采用。

保留两个明确入口：

- **导出照片…**：主操作，全尺寸重新渲染，采用本次点击导出时的参数快照。
- **保存当前预览…**：次操作，复用已有渲染缓冲区。使用同一套格式选项，标题、尺寸、参数版本明确标注为当前预览；新参数尚未渲染时显示“此预览未包含最新调整”，只允许保存所见旧预览，不能假装是最新成片。没有缓冲区时禁用。

原有 Save bit depth、Export backend、Saving color space/CCTF 的编辑入口迁入弹窗；主界面可以显示摘要，不保留第二套可编辑状态。项目/配方的 Save state 继续独立。

## 调研证据

当前源码：

- `spektrafilm-gui/src/main.rs` 的 `save_dialog` 保存已有缓冲区；`export_dialog` 调用 `digested_params(false)`，以后台子进程全尺寸重算。格式靠文件扩展名识别，默认文件名是 PNG。
- 同文件 `import_section` 将位深、CPU/GPU 与路径选择分开；`run_export` 传位深和保存颜色参数，没有 JPEG 质量/采样参数。已有取消与同目录暂存后发布机制，可复用。
- `spektrafilm-core/src/image_io.rs` 支持 JPEG/PNG/TIFF/EXR；目前 JPEG/PNG 固定 8-bit，TIFF 8/16/32，EXR half/float32。`convert_image` 做保存色彩转换；相同空间及编码是无损直通。
- `image_io_bridge.cpp` 已修复 JPEG 质量属性为 `CompressionQuality`；未设置采样时，本机 OIIO 2.5.19.1 实测为 4:2:0。TIFF 当前固定 ZIP，EXR 压缩依赖编码器默认值。EXR 没有现成的可靠颜色原色标签写入链路。
- 本次颗粒实验：默认 JPEG 会损失彩色细节。质量数值升高不代表关闭色度降采样，也不能保证像素梯度指标单调改善。

产品参考：

- [darktable export](https://docs.darktable.org/usermanual/development/en/module-reference/utility-modules/shared/export/)：格式决定可见选项，质量默认 95；路径冲突策略、输出色彩和后台取消在导出流程内。
- [Lightroom Classic export](https://helpx.adobe.com/lightroom-classic/desktop/export-photos/export-files-disk-or-cd.html)：位置、命名、格式、位深及颜色设置集中在导出窗口。

借鉴集中设置与条件显隐。本设计只解决单张照片导出；不增加批量队列、命名模板语言、云发布、导出缩放、输出锐化和文件大小预算。

## 弹窗结构

建议桌面宽 680–760 px，高度不超过工作区 85%，内容可滚动，底部动作固定。视觉沿用现有 egui 深色面板。

1. 标题「导出照片」；源文件名、胶片、实际导出宽高；说明“按当前裁切与旋转重新渲染”。此处全尺寸指现有管线几何处理后的尺寸，包含已设置的 upscale，不强行等于输入尺寸。
2. 文件：文件名主体 + 只读格式扩展名；目标文件夹 +「选择…」。实际实现调用原生目录选择器。默认使用最近成功导出的可访问目录；没有记录则输入文件目录。默认主体为 `{原图名}_{胶片名}`，清理路径非法字符。
3. 格式：JPEG / TIFF / PNG / OpenEXR；在同一位置呈现格式专属选项。
4. 颜色空间：当前有效值直接可见，不藏在高级设置。JPEG/PNG 首次 sRGB；TIFF 首次沿用当前输出空间和编码；EXR 首次线性 ACES2065-1。
5. 高级：渲染方式、TIFF 浮点编码等。渲染精度不与 JPEG 压缩质量共用“质量”标签。
6. 底部摘要：文件名、格式、尺寸、位深、颜色空间；按钮「取消」「导出照片」。无真实编码结果前不显示精确文件大小或预计耗时。

初次默认 JPEG / 95 / 4:4:4 / sRGB / CPU。已有用户优先迁移已有保存颜色、位深和 backend；JPEG 新字段用上述默认。之后记住最近成功提交的格式设置，各格式分别记忆；关窗取消不写持久化偏好。

## 格式能力矩阵

| 格式 | 位深与默认 | 可见编码选项 | 默认颜色规则 | 文件名 |
|---|---|---|---|---|
| JPEG | 固定 8-bit RGB | 质量整数 1–100，默认95；色彩细节 4:4:4 / 4:2:0，默认4:4:4 | sRGB，显示编码；可选现有有ICC的显示编码RGB空间 | .jpg |
| TIFF | 8 / **16整数** / 32浮点 | ZIP无损 / 不压缩，默认ZIP | 沿用当前输出空间；整数使用显示编码，浮点可选显示编码或线性 | .tif |
| PNG | 固定8-bit RGB | 无损压缩，无“质量”滑块 | sRGB，显示编码；可选有ICC的显示编码空间 | .png |
| OpenEXR | **16浮点half** / 32浮点 | ZIP无损，固定显示 | 线性；默认ACES2065-1，选择支持写入原色标签的空间 | .exr |

PNG 16-bit 不在当前实现范围；UI 不展示一个实际仍写8-bit的选项。EXR half 的量化仍存在，“ZIP无损”只指压缩不增加额外损失。

JPEG 色彩细节文案：

- **保留完整色彩细节（4:4:4）**：保留彩色颗粒与细小色彩边缘，文件通常更大。
- **较小文件（4:2:0）**：减少色彩采样，细小彩色颗粒可能变软。
- 质量100提示：“仍是有损 JPEG。无损保存请选择 TIFF 或 PNG。”

不得因为用户提高 quality 就自动切换 subsampling，两者独立。4:2:2 不增加到此次 UI。

颜色：JPEG/PNG 可选 sRGB、Display P3、Adobe RGB、ProPhoto RGB、BT.2020、DCI-P3，均使用已存在的对应显示编码ICC；ProPhoto/广色域8-bit给予简短色阶提示，不阻断。TIFF 32浮点允许线性sRGB、AdobeRGB、ProPhoto、BT.2020、ACES2065-1；现有缺失ICC的线性P3不开放。EXR只开放线性sRGB、ACES2065-1，必须先实现标准chromaticities标签，不允许仅写一个空间名字。JPEG/PNG不可出现线性开关。切换格式使用该格式上次合法配置，不默默改写另一格式的配置。

保存颜色转换不能改变胶片模拟的输出空间参数，不能重新配置颗粒算法。整数格式会截断[0,1]外数值，摘要中明示；浮点格式保留编码后可表示的负值及高光。避免为修正色差偷偷新增 gamut mapping。

## 路径、任务与状态

文件名输入只包含主体；粘贴.jpg/.jpeg/.tif/.tiff/.png/.exr后缀时移除已知后缀，由选定格式统一生成；其它点号作为主体保留。拒绝空名、分隔符、平台保留名；平台校验由共享后端执行。原文件绝不作为可覆盖目标。

提交时固定输入身份、处理参数、随机seed、旋转/裁切、输出设置和目标路径。导出期间用户继续调色不影响该任务。每次只运行一个导出，不新增队列。

状态转换：编辑 → 校验 → 同名确认（如有） → 准备/渲染/写入 → 发布 → 完成；运行中可进入取消中；任一步可进入失败。

- 同名时显示完整路径，可选“另存副本”“替换”“返回”。不把一次替换批准当作未来导出的全局授权。若处理期间目标变化，发布前重新确认；不自动覆盖新文件。
- 临时输出放目标目录。只有写入、关闭成功且确认未取消后才原子发布。失败/取消删除本次临时文件，原目标保留。
- 原生编码不可中断的阶段显示“正在取消，等待当前写入结束”；清理完成前不显示已取消。
- 弹窗可收起到任务条，任务仍运行；任务条显示文件名、真实阶段和已用时，可展开、取消。没有后端进度数据时只显示不定进度，不生成假百分比。单击关闭主窗口沿用取消并回收子进程行为。
- 完成显示真实尺寸、格式、文件大小、路径以及“在文件夹中显示”。不自动打开外部应用。
- 失败保留设置，显示可读原因及“重试”“返回设置”。磁盘空间不足、目录不可写、源文件丢失、缺少导出程序分别说明。
- 元数据写入失败但像素成功：显示“已导出，元数据未完整写入”，不能当作无条件成功或删除有效照片。

保留现有默认元数据复制行为，UI 简述“保留源 EXIF/IPTC/XMP（可能包含位置）”；EXR明确不复制这些字段。此次不扩展隐私过滤开关，ICC/EXR chromaticities始终跟随实际像素颜色。

CPU 默认沿用f64；高级中允许GPU f32，但标明结果可能存在数值差异。导出前检查适配器、尺寸及各缓冲区限制；不支持时给出“改用CPU导出”，不让程序因已知大小限制崩溃，也不静默改变后端。没有通过真实照片尺寸验证的GPU路径不得显示为推荐项。

## 参数记忆、可访问性与原型

每种格式独立记忆合法设置；切换到PNG隐藏JPEG参数但不清空其95/4:4:4选择，切回恢复。用户编辑的文件名主体不随格式改变，扩展名同步。取消弹窗恢复打开时配置，已提交任务的快照不受影响。

Tab按阅读顺序移动；Escape在编辑时取消，在任务状态下只收起，不取消任务；Enter在编辑有效时提交。格式变化保留焦点并更新可读摘要。错误紧贴字段，不只靠颜色；窄窗口滚动内容，动作栏保持可达。

原型提供JPEG/TIFF/PNG/EXR联动、格式记忆、文件名验证、提交摘要、任务收起、取消和失败重试。原型的目录由文本输入模拟，完成/失败由独立的“原型场景”控件触发；不假装浏览器能够写入指定本机文件夹。

## 实现边界

本次交付设计，不实现原生功能。下列 CLI 参数是拟议契约，尚不能在当前二进制执行；不新增配方 DSL。GUI/CLI 必须共享完整导出能力，不能只在 GUI 扩展选项。

- GUI：从`main.rs`抽出导出弹窗和草稿状态；复用现有worker/cancel/staged publish。输出设置只保留一个状态来源，迁移旧GUI状态中的save_bit_depth/export_backend/saving字段。
- Core I/O：共享格式能力校验；编码设置按格式建模（JPEG质量和采样、TIFF位深和压缩、PNG8、EXR位深）。正常调用者不传无关字段，外部非法组合报错。`save`与显式JPEG质量路径统一到同一编码实现，迁移所有调用者，不留下两套不同默认策略。
- Native：显式设置CompressionQuality、jpeg:subsampling、TIFF压缩、EXR ZIP及chromaticities。EXR须由原色坐标、白点推导标签，与颜色转换一致。
- CLI process：提供下表全部全尺寸导出选项，GUI生成显式参数并调用同一执行路径。现有结构化render的quality85合约保持其明确语义，不作为通用导出的功能上限；它与process复用编码实现。新增轻量encode入口承接已有渲染图编码，与GUI保存预览等价。
- Export程序：启动前检查版本/能力，避免GUI选项被旧CLI拒绝或忽略；导出阶段/取消/元数据警告传回GUI。用户可读界面无需暴露协议细节。

## GUI / CLI 等价契约

“等价”指相同输入像素、参数快照、seed、后端与导出设置，生成相同尺寸、编码属性、颜色含义及解码像素；不要求包含写入时间的文件逐字节相同。GUI 的记忆值必须显式传入CLI，CLI不能隐式读取GUI偏好。相同平台及后端的确定性输出应逐像素一致；CPU/GPU之间单独按已有精度标准衡量。

| GUI 选项 | 拟议 CLI 参数 | 统一默认/规则 |
|---|---|---|
| 导出照片 | `process INPUT -o OUTPUT --params SNAPSHOT` | 原图重新渲染，忽略交互预览开关，以当前裁切/旋转/upscale得到完整尺寸 |
| 保存当前预览 | `encode RENDERED -o OUTPUT --input-color-space SPACE --input-cctf-encoding true\|false` | 已渲染像素只做颜色转换和编码；不再次胶片处理，不缩放。输入空间/编码必填，不猜测 |
| 文件格式 | `--format jpeg\|tiff\|png\|exr` | 缺省由OUTPUT合法后缀识别；显式格式与后缀冲突时报错，接受jpg/jpeg、tif/tiff别名 |
| JPEG质量 | `--jpeg-quality 1..100` | JPEG默认95，整数；非JPEG显式传入时报错 |
| JPEG色彩细节 | `--jpeg-subsampling 444\|420` | JPEG默认444，独立于quality；非JPEG拒绝 |
| 位深 | `--bit-depth 8\|16\|32` | 缺省依格式为JPEG8、PNG8、TIFF16、EXR16；不合法组合报错，禁止静默降位深 |
| 压缩 | `--compression zip\|none` | TIFF默认zip，可选none；EXR只接受zip；JPEG/PNG不接受此参数 |
| 保存颜色空间 | `--saving-color-space SPACE` | JPEG/PNG初始sRGB；TIFF继承渲染输出；EXR初始ACES2065-1；使用同一可支持空间集合 |
| 像素编码 | `--saving-cctf-encoding true\|false` | JPEG/PNG及整数TIFF必须true，EXR必须false，浮点TIFF继承渲染输出；不合法组合拒绝 |
| 渲染方式 | `--backend cpu\|gpu` | process默认CPU f64，GPU f32显式选择；encode不渲染，拒绝backend参数 |
| 同名文件 | `--on-conflict error\|rename\|replace` | CLI默认error，无交互挂起；GUI首次遇到冲突进入确认，对应用户选择再调用相同策略 |
| 导出状态/取消 | stderr真实阶段与耗时；Ctrl+C | GUI消费相同任务事件并显示任务条；SIGINT走取消和清理，非零退出且不发布半成品 |

初次GUI默认JPEG95/444与CLI JPEG缺省值一致。已迁移或已记忆的GUI设置不是新的引擎默认值；生成命令应完整展开。`process` 原来的 `--bit-depth` 全局默认16要改为按格式解析；兼容的旧命令继续按后缀工作，明确非法组合从“忽略”改为报错并在帮助/变更记录注明。当前通用JPEG质量和采样依赖编码器默认值；改为显式95/444是产品选择，不声称文件大小或视觉质量严格单调变化。

示例（拟议命令，文件路径仅示例）：

```sh
spektrafilm-f64 process photo.tif -o photo_portra.jpg \
  --params export-params.json --backend cpu --format jpeg \
  --jpeg-quality 95 --jpeg-subsampling 444 --bit-depth 8 \
  --saving-color-space sRGB --saving-cctf-encoding true --on-conflict error

spektrafilm-f64 process photo.tif -o photo_portra.tif \
  --params export-params.json --backend gpu --format tiff \
  --bit-depth 16 --compression zip \
  --saving-color-space 'ProPhoto RGB' --saving-cctf-encoding true --on-conflict rename

spektrafilm-f64 encode rendered-preview.tif -o preview.jpg \
  --input-color-space sRGB --input-cctf-encoding true \
  --format jpeg --jpeg-quality 95 --jpeg-subsampling 444 \
  --saving-color-space sRGB --saving-cctf-encoding true --on-conflict error
```

encode输入必须是实际保存的渲染缓冲区；想重现GUI旧预览，就使用同一缓冲区的浮点文件。它不等于用最新参数重新生成预览。主GUI提供“查看等价命令”文字摘要用于核对设置；完整可移植重现必须同时保存输入解释、RAW选项、处理快照及seed，不能给出引用已被删除临时文件的伪可复现命令。此次原型只展示拟议选项映射，不伪造可运行文件。

GUI与CLI都在编码/渲染前校验输出组合及可预知路径错误。元数据写入警告同样保留有效照片并显式报告；未知空间、设备不可用、编码失败不能假装成功。CLI成功返回0，失败非零，取消用约定的中断退出码；阶段协议在实施时沿用现有程序约定，不另建第二套任务调度。

原子覆盖和另存副本由共享执行层处理：rename安全预留唯一目标，replace按已确认目标身份检查，不能把检查和发布之间的竞争条件交给GUI。两端同样禁止覆盖输入原文件。

等价验收必须从两个入口真实运行：固定输入/配方/seed/CPU或GPU，遍历四格式合法位深、JPEG质量和采样、TIFF压缩、编码空间及冲突策略；比较文件头、量化表、采样因子、ICC/chromaticities与解码像素。GUI调色过程与任务快照隔离、CLI SIGINT与GUI取消的文件结果一致。反例包括PNG16、JPEG线性、EXR8、非JPEG传quality、扩展名冲突、GPU初始化失败：GUI不给非法选择，CLI明确报错，共享校验拒绝任何绕过UI的请求。


## 验收标准

1. JPEG质量1/95/100实际改变编码量化；读取输出JPEG量化表和像素，不能只检查传参。4:4:4与4:2:0读取SOF采样因子确认。
2. TIFF8/16/float32、PNG8、EXRhalf/float32的文件头及回读精度符合选择；EXR无8-bit选项，PNG不继承TIFF的16-bit展示。
3. JPEG→TIFF→JPEG恢复质量和采样；扩展名与编码一致；无效文件名无法提交；取消草稿不污染上次设置。
4. 低分辨率预览存在、最新调色尚未渲染时，导出仍使用最新参数的全尺寸处理；保存预览明确使用已有快照。至少一个真实照片尺寸场景验证。
5. sRGB/P3/ProPhoto输出做颜色管理回读；EXR的线性像素和chromaticities一致；整数裁切与浮点保留边界符合说明。无意外二次编码或颗粒后缩放。
6. 同名拒绝/替换、副本、发布前目标变化、磁盘失败、运行中取消都不损坏原目标；已取消任务不再发布文件。
7. 后台导出、收起、重开、继续调色、失败重试、缺CLI、GPU尺寸不支持均实际运行验证。无伪百分比。
8. Windows/macOS/Linux的原生目录选择和文件替换行为验证；用键盘完成全流程；弹窗在窄窗口可滚动且动作可达。

设计验证只证明原型交互与规格一致，不能替代以上原生编码及GUI验收。

## 本次设计验证记录

使用真实Chromium打开本地原型，检查桌面及390×844窄窗口截图；动作栏可见，内容区独立滚动。已操作验证：JPEG88/420切TIFF32线性，再切PNG8、EXR16线性，切回JPEG仍为88/420；非法文件名禁用提交；CLI摘要随EXR/JPEG/保存预览入口更新；模拟失败→重试→取消中→取消完成；收起任务→重新打开→模拟完成。原型明确标注示例尺寸和模拟任务。

未运行原生GUI/CLI编码验收，未验证真实文件写入、路径冲突和元数据。HTML中的拟议命令不能直接当作当前CLI可用参数。
