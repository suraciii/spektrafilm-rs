# Dehancer 颗粒渲染逆向分析

> 目标：下载 Dehancer 官方发行版，对其程序做静态逆向，还原其 Film Grain（颗粒）渲染的完整实现，作为 SpektraFilm 颗粒模块设计与学习的参考。
>
> 分析对象：`DehancerPro_InstallerDavinciResolve_Linux_7.4.1.tar.gz`（官方 CDN 直链下载，2026-06-30 构建）
> 内含 OFX 插件 `DehancerProOpenCL_x86_64_v7.ofx`（161 MB，stripped ELF x86-64，内嵌 OpenCL kernel 源码与 cereal JSON 配置）。
>
> 本文档只记录算法行为、参数语义与公式（以自有伪代码/公式表述），不含 Dehancer 源码原文。

> 第 1–13 节保留早期调查记录，其中 Noise 滤波、零 Amount、Film Type 路由与固定 Rec.709 包装等结论已由第 14–15 节的宿主证据取代；当前集成以末尾的实现与验证记录为准。

## 0. 逆向方法与证据来源

| 证据 | 位置 | 说明 |
|---|---|---|
| 颗粒配置 JSON | `bundle/Contents/Resources/profiles/grain/index_film.json` | 明文 cereal 序列化，12 条颗粒 profile 的全部参数与取值范围 |
| OpenCL kernel 源 | `.ofx` 二进制 `.data` 段内嵌的预处理后 `DehancerKernel.cl`（约 212 KB 单块，含 CI 路径 `framework/shaders/shared/FilmGrain.h / GrainUtils.h / ScanGrain.h`） | 颗粒生成/合成全部 GPU 代码 |
| 宿主端 C++ | 二进制虽 stripped，但导出符号保留：`dehancer::FilmGrainKernel::{process,update_state,make_grain_texture,make_grain_desc,get_grain_scale_factor,get_resolution_radius_legacy}`、`dehancer::FilmGrainGenerator::{setup,update_options}`、`dehancer::impl::FilmGrainKernelImpl::setup`、`dehancer::ScanGrainKernel::update_state`、`dehancer::states::{Segment,Limits}::get_value/...` | 参数换算数学全部出自这些函数的反汇编 |
| 官方文档 | dehancer.com/learn/article/grain、/learn/articles/how-does-film-grain-work-in-dehancer-ofx-plugin | 参数语义的官方口径，与逆向结果互相印证 |

关键事实（官方文档口径，逆向证实）：**"Grain size is automatically corrected based on the geometric size of the image"** —— 颗粒尺寸按图像几何尺寸自动换算，与分辨率无关。

## 1. 总体架构

Dehancer 的颗粒不是叠加噪声贴图，而是三段式管线（`FilmGrainKernel::process` 反汇编还原）：

```
输入图像（已过 film profile / print 阶段）
   │
   ├─① Film Resolution 预模糊（可选，默认 profile 均开启）
   │     resolution_type=1 → FastBlur（精确 Gaussian，水平/垂直各一遍）
   │     resolution_type=0 → OpticalResolution（整数 tap 平顶重采样核）
   │
   ├─② 颗粒纹理生成 FilmGrainGenerator（在"虚拟胶片画布"分辨率上）
   │     kernel_film_grain_generator(_8bit)
   │     输出 grainTexture（尺寸 ≈ 图像 × 画布比例，见 §3）
   │
   └─③ 颗粒重采样合成 FilmGrainKernelImpl
         kernel_film_grain_resampler
         9 邻域采样 grainTexture → 分区(暗/中/高) Overlay 混合进图像
```

另有直接在图像分辨率上算噪声的 `kernel_scan_grain`（`ScanGrainKernel`），对应 **Digital (Experimental)/Noise 模式**：不执行 Film Resolution 预模糊，直接从输入图像生成并合成噪声。

## 2. 颗粒 profile 数据（全部 12 条）

`canvas_size = 5200×3100`（**所有格式共用同一虚拟画布**），`cluster_size = 1.6`，`rotation = 1.0`，`is_colored/is_clustered = true`，`resolution_type = 1`，`type = 0`（负片型）：

| profile | scale | amount | shadows | mid | high | color(chroma) | resolution_factor |
|---|---|---|---|---|---|---|---|
| 8mm50   | 40 | 25 | 30 | 50 | 50 | 50 | 50 |
| 8mm250  | 48 | 50 | 25 | 45 | 65 | 65 | 70 |
| 8mm500  | 48 | 70 | 30 | 45 | 65 | 90 | 75 |
| 16mm50  | 25 | 25 | 30 | 55 | 65 | 65 | 65 |
| 16mm250 | 25 | 45 | 30 | 55 | 65 | 65 | 75 |
| 16mm500 | 25 | 70 | 30 | 45 | 65 | 75 | 80 |
| 35mm50  | 8  | 30 | 30 | 55 | 65 | 65 | 70 |
| 35mm250 | 12 | 35 | 30 | 55 | 65 | 65 | 75 |
| 35mm500 | 16 | 40 | 35 | 55 | 65 | 70 | 80 |
| 65mm50  | 1  | 10 | 30 | 45 | 55 | 65 | 90 |
| 65mm250 | 2  | 15 | 30 | 55 | 65 | 65 | 90 |
| 65mm500 | 3  | 25 | 25 | 55 | 65 | 80 | 100 |

取值范围：scale 1–48，amount/分区 0–100，resolution_factor **100–0（反向：值越小越模糊）**。
注意量纲：**scale 是"画布像素/颗粒"的倒数概念**——8mm 胶片颗粒大 → scale 大；65mm 颗粒细 → scale 小。分辨率越低的胶片（8mm）ISO 越高时 resolution_factor 越低（原始细节更早被颗粒吞掉）。

## 3. 分辨率无关的核心：虚拟画布 + gsf（这是本次逆向最重要的结论）

宿主端 `get_grain_scale_factor()`（反汇编还原）：

```cpp
float gsf = max(canvas.w * viewport.w / tex.w,      // 5200
                canvas.h * viewport.h / tex.h);     // 3100
```

`make_grain_desc()`：

```cpp
grain_w = min(max_texture_size, image_w * gsf);     // 高度同理（各有 −0.2 取整微调）
```

语义：

1. **颗粒定义在固定 5200×3100 的"胶片画布"上**，与渲染分辨率完全解耦。
2. 颗粒纹理实际尺寸 = `图像尺寸 × gsf`（宽高同乘 gsf），再被 GPU 最大纹理尺寸封顶。1920×1080 图像 → gsf = max(5200/1920, 3100/1080) = 2.87 → 颗粒纹理 ≈5510×3100；3840×2160 图像 → gsf = max(1.354, 1.435) = 1.435 → 同样 ≈5510×3100。**任何分辨率、任何宽高比下颗粒纹理都被归一到同一张"虚拟画布"**。
3. 生成器里噪声单元（cell）尺寸 = `cluster_size × scale`（画布像素，恒定）。
4. 重采样器把画布映射回图像（见 §5）。于是**同一画面在 1080p 和 4K 输出下，颗粒的"像素尺寸"随图像等比放大，而相对画面的尺寸不变** —— 正是真实胶片以不同 DPI 扫描的行为。

SpektraFilm 现状对照：`grain.blur`（最终高斯 σ）与 `blur_dye_clouds_um`、`micro_structure` 都以**绝对像素/微米**为单位直接作用在输出分辨率上，高分辨率图像上颗粒相对变小变碎（用户观察到的"小格状"），根因就在这里。

## 4. 颗粒纹理生成器（kernel_film_grain_generator）

参数（`FilmGrainGenerator::setup` 设置，`Options` 由 `update_state` 产出）：`is_clustered(uint)、cluster_size(float)、scale(float)、colored(uint)、color(float)、rotation(float)、timer(float)`。

`update_state` 的换算（反汇编）：

```
gen_scale = clamp(state.scale.value, 0.5, 1.4)     // 生成器一侧的 scale 被夹在 [0.5,1.4]
color     = is_colored ? Segment(color).get_value() : 1.0
```

kernel 内部逻辑（还原伪代码）：

```glsl
width  = tex.size.x * scale;            // tex = 颗粒纹理（≈画布）
pos    = gid / (width, height);         // 归一化坐标（span 1/scale）
mult   = width / cluster_size / scale;  // = 画布宽 / cluster_size
texel  = 1/256 / cluster_size;          // 噪声哈希网格量化步长

// 每通道不同的旋转角（簇状时用 4D snoise 随像素扰动）
rotOffset = clustered ? snoise4(timer, pos) * rotation
                      : (1.425, 3.892, 5.835) * rotation * scale;
rotCoords = coord_rot(pos, rotOffset.ch, width, height);  // 宽高比校正的坐标旋转

p = rotCoords * mult;                   // 噪声坐标 = gid/(scale·cluster_size)
grain1 = pnoise3D(p, timer,     texel); // z=0 场
grain2 = pnoise3D(p, timer/2,   texel); // z=1 场（时间减半 → 第二相位）
grain  = mix(grain1, grain2, pointLum); // 按输入亮度在两个场之间插值 ←—"图像由颗粒构成"的关键之一

if (colored) {
  grain.g = mix(grain.r, pnoise3D(rot(coordG, mult), timer, texel), color);
  grain.b = mix(grain.r, pnoise3D(rot(coordB, mult), timer, texel), color);
}
write(grain + 0.5);                     // 输出到 [0,1]，0.5 为中性
```

要点：

- **噪声本体是 3D 值噪声**：`rnm()` 用经典 `sin(dot(tc+timer, (12.9898,78.233)))×43758.5453` 哈希生成梯度向量，`fade(t)=6t⁵−15t⁴+10t³` 五次插值三线性插值——即 Shadertoy 圈流传的 "pnoise3D"（textureless procedural noise），Dehancer 把第三维 z 用作固定相位、`timer` 用作动画相位。
- **每个像素的最终噪声 = 两个独立噪声场按该点亮度混合**：暗部用场 1、亮部用场 2，颗粒结构与图像内容相关（官方文档所谓 "generates grain based on local colour and brightness"）。
- **彩色颗粒**：G/B 通道用不同旋转角的坐标再采两个独立场，与 R 按 `color`（chroma 0–1）混合；`color=0` 时三通道相同（黑白颗粒）。
- **簇（cluster）**：`is_clustered` 时旋转角本身被 4D noise 扰动 → 颗粒方向/密度在空间上成团，叠加 `cluster_size=1.6` 的基础单元，形成"颗粒团块"而非均匀网点。
- **timer**：宿主端用 MT19937 每次渲染生成随机相位（视频逐帧演变；照片模式下固定）。`snoise`/`rnm` 哈希都吃 timer。
- 生成器有 `_8bit` 变体（8bit 纹理走 `kernel_film_grain_generator_8bit`）。

## 5. 颗粒合成（kernel_film_grain_resampler）

`FilmGrainKernelImpl::setup` 传入的标量参数（反汇编）：

```
arg3 scale     = Limits(scale).get_normalized_value()      // 见下
arg4 amount    = Segment(amount).get_effective_value()
arg5 shadows   = Segment(shadows).get_effective_value()
arg6 midtones  = Segment(midtones).get_effective_value()
arg7 highlights= Segment(highlights).get_effective_value()
arg8 overscan/damage mask 纹理（无则 1×1）
```

`states::Segment`：`get_value() = (v−min)/(max−min)+min`（0–100 → 0–1 归一化）；
`get_effective_value() = 0.12·t² + 0.68·t + 0.2`（t 为归一化值）——**分区强度带一条提升低端的响应曲线**。
`states::Limits`：`{value,min,max,low,high}`，`get_normalized_value() = (v−min)/(max−min)·(high−low)+low`。scale 的 low/high 由默认构造给出 **1.0 / 2.5**，即：

```
resampler_scale = 1.0 + (scale−1)/47 × 1.5   ∈ [1.0, 2.5]
```

| profile scale | resampler_scale |
|---|---|
| 1 (65mm50) | 1.00 |
| 12 (35mm250) | 1.35 |
| 25 (16mm) | 1.77 |
| 40 (8mm50) | 2.25 |
| 48 (8mm250) | 2.50 |

kernel 逻辑（还原）：

```glsl
inputCoord = gid / (outSize − 1);                 // 图像归一化坐标
size    = grainTexSize / scale;                   // 图像覆盖画布的 1/scale 区域
coord   = int2(inputCoord * size);

grain = (1/9)·Σ_{3×3} read(grainTex, coord+δ);   // 9 邻域均值 = 降采样抗混叠滤波

if (damage/overscan mask 有效) {
    grain *= mask.rgb;                            // 损伤蒙版乘法
    highlight_mask = luma(mask);                  // 并调制高光区不透明度
}
luma = dot(rgb, (0.2125, 0.7154, 0.0721));        // Rec.709 亮度

op_sh = exp(−0.5·((luma−0.0)/0.2)²);              // 三个分区权重：σ=0.2 的高斯钟
op_mid= exp(−0.5·((luma−0.5)/0.2)²);
op_hi = exp(−0.5·((luma−1.0)/0.2)²) · highlight_mask;

rgb = mix(rgb, overlay(pow(rgb, 0.8), grain), shadows  · amount · op_sh  · 2.0);
rgb = mix(rgb, overlay(rgb,           grain), midtones · amount · op_mid · 1.0);
rgb = mix(rgb, overlay(rgb − 0.2,     grain), highlights· amount · op_hi  · 2.0);

// overlay = Photoshop 叠加：base<0.5 ? 2·base·blend : 1−2·(1−base)(1−blend)
// 输出以 alpha=0.5 过 blend_normal 半透明合成回原图
```

要点：

- **scale 即画布→图像的缩放倍率**：图像只覆盖颗粒纹理的 `1/scale` 区域并放大显示。scale∈[1,2.5]：65mm 细颗粒 scale=1（整幅画布映射到图像，2–3× 超采样平均后颗粒细腻）；8mm 粗颗粒 scale≈2.5（画布中心区域放大 2.5×，颗粒单元成比例变大）。
- **颗粒以 Overlay（叠加）模式、乘以分区权重后混入**，且三个分区分别做了基底修正：暗部 `pow(rgb,0.8)`（提亮基底让 Overlay 在暗部可见）、高光 `rgb−0.2`（压暗基底避免高光过曝放大）；暗部/高光系数 ×2、中间调 ×1。
- **分区权重是 σ=0.2 的三个高斯钟**（中心 0 / 0.5 / 1），互相有交叠 → "Shadows/Midtones/Highlights" 三个滑杆的实际作用域。
- **Overlay 的 0.5 中性点只针对未经修改的 base**；完整合成含暗部 `pow0.8` 和高光 `−0.2`，因此 grain=0.5 仍可改变图像均值，不能把最终图像的均值变化全部计作随机颗粒。
- 最终 `result.w=0.5; blend_normal(in, result)` —— 颗粒层以 50% 不透明度叠加，等效强度减半。
- **Digital 模式（kernel_scan_grain）**：无独立纹理，`r = rotCoords × (W/scale, H/scale)`，`texel = cluster_size/256`，其余分区混合公式完全相同；`timer` 额外被 `snoise(timer, inColor)` 抖动。用于性能优先场景。

## 6. Film Resolution（分辨率耦合预模糊）

`FilmGrainKernel::process` 开头（反汇编还原）：

```cpp
float a   = amount.get_value();                    // 0–1
float amount_factor = 0.7·a² + 0.3·a + 0.05;
float res_norm = resolution_factor.get_normalized_value();   // 反向 limits 100→0
float s_norm  = Segment{scale.value, scale.min, scale.max}.get_value(); // = (s−1)/47+1 ∈[1,2]

float radius = max(0, s_norm · res_norm / gsf)
             · K[resolution_type]                  // K = {1.6 (optical), 1.2 (fast)}
             · amount_factor;

if (radius > 0) {
    if (resolution_type == 1)  FastBlur(src, dst, radius);           // Gaussian folded bilinear taps
    else                       OpticalResolution(src, dst, radius);  // integer-tap resampler
}
```

（历史版本 `get_resolution_radius_legacy` 用 `×1.87` 与 amount 的 effective 曲线，现行主路径如上。）

语义与官方文档完全对应："胶片上最小细节不会小于颗粒尺寸"。**把源图预先模糊到颗粒尺度**，模糊半径同样除以 gsf → 分辨率等比；resolution_factor=100 保留原始清晰度，50 为"细节与颗粒平衡"，0 全糊。`resolution_type=1`（所有 profile 默认）走精确 Gaussian FastBlur；`=0` 走整数 tap OpticalResolution。Noise/Digital 路径不使用 Film Resolution。

## 7. 颗粒类型（Negative / Positive）与 Expand

- `type`、`Film Type` 与 `resolution_type` 是不同概念。当前 12 个 profile 的 JSON 为 `type=0`（负片）和 `resolution_type=1`；前者是 profile 元数据，后者才决定 Film Resolution 算法。SpektraFilm 不再让 Negative/Positive 选择 OpticalResolution/FastBlur。
- 官方文档提示：颗粒影响黑白场（Overlay 在 0/1 处仍会推动），需要 **Expand** 工具恢复对比度——与 SpektraFilm 的 density_min/expand 概念同源。

## 8. 与 SpektraFilm 的对照与可借鉴设计

| 维度 | Dehancer | SpektraFilm 0.3.4 现状 | 建议 |
|---|---|---|---|
| 尺寸基准 | 虚拟画布 5200×3100，gsf=画布/图像，颗粒单元=画布像素 | `particle_area_um2`/`pixel_size_um` 物理微米模型（物理正确但依赖 scan 像距假设） | 保留物理模型，但**渲染时引入 gsf 归一化**：σ_px = σ_film × W/canvas_w，让颗粒随输出分辨率等比缩放 |
| 最终模糊 | 无"最终 blur"参数；颗粒大小由 scale+cluster_size 在画布上定义，重采样 9 邻域均值滤波 | `grain.blur` 以输出像素为单位的固定 σ（0.65）→ 分辨率相关，高分辨率下颗粒碎、低分辨率下糊 | 把 `grain.blur` 语义改为画布/物理单位（如 σ_um × gsf 或 σ × W/ref_w），或加 reference_width 参数 |
| 分辨率耦合 | Film Resolution：按颗粒尺度预模糊源图，radius/gsf 分辨率等比 | 无对应（scanner.unsharp 是全局的） | 可选新增：detail-limit 预模糊 σ = f(grain_size, resolution_factor)·W/canvas_w |
| 亮度分区 | 三高斯钟 σ=0.2 @ luma{0,0.5,1}，Overlay 混合，暗部 pow0.8/高光 −0.2 基底修正 | Poisson-binomial 物理颗粒密度涨落（更物理） | 物理模型可保留；分区控制可参考其响应曲线 0.12t²+0.68t+0.2 与 σ=0.2 钟形 |
| 彩色颗粒 | 每通道独立旋转坐标 + chroma 混合 | per-channel particle_scale + monochrome flag | 已同构 |
| 动画 | timer 随机相位逐帧演变 | 静态 seed | 视频用途时可选 |
| 颗粒观感 | Overlay(0.5 中性) 半透明 → 双向密度调制 | 密度域加性涨落（均值保持） | 两者原理一致（密度涨落）；Dehancer 的 Overlay 是显示域近似 |

**最直接可落地的三条**：

1. **gsf 归一化**：`scale_px = value_um / pixel_size_um` 之外，增加 `render_ref_width`（默认 5200 或物理 36mm 对应像素），一切以像素为单位的 σ（grain.blur、micro_structure blur、unsharp）乘以 `W/render_ref_width`。这解决"高分辨率照片处理后颗粒变成小格/模糊参数不适配"的直接痛点。
2. **颗粒分辨率耦合模糊**（对应 Film Resolution）：`σ_detail = k · grain_size_px · (1 − resolution_factor)`，同样 gsf 归一化——让源图细节不超过颗粒尺寸，颗粒观感从"贴图噪声"变为"成像介质"。
3. **分区响应**：如需 UI 分区控制，σ=0.2 的三个 luma 钟形 + 各区独立 strength 是经验证好用的交互模型。

## 9. 遗留不确定项

- `resolution_factor` Limits 的 low/high 归一化端点在反汇编中未完全定位（构造默认值疑似 0.1/0.1，OFX 层可能动态覆写）；其定性语义（100=不模糊→0=全糊、半径÷gsf）已由官方文档+主路径公式交叉证实。
- 生成器 `clamp(scale, 0.5, 1.4)` 与重采样器 `[1.0, 2.5]` 归一化端点并存：生成器夹紧只影响噪声单元基础尺寸（≈2.24 画布像素恒定），格式间大小差异由重采样 scale 承担——此解释与全部反汇编证据自洽，但未做动态运行验证。
- `OpticalResolution` 的 PSF 权重生成在宿主端，本次只确认了卷积 kernel 形态（分离、CLAMP/BORDER/WRAP 三种边界）。

## 10. 工件位置（本机）

- 当前研究仓库：`/home/szf/repos/dehancer-re/`。
- 色彩域证据：`report/dehancer-color-domain.md:21–40`（Photo `by_pass` 保持输入显示编码；并未证明所有输入的唯一 transfer）。
- 核函数证据：`artifacts/DehancerKernel_complete.cl`：亮度分区/Overlay 5073–5091、原 `snoise` 5102–5127、Analogue 5340–5385、Noise 5425–5472；Rec.709 分段 transfer helpers 位于 3944–3970。
- 原宿主二进制与反汇编工具：该仓库 `plugin/` 和 `tools/dis_f.py`。

以上原始工件只读参考，不复制到本仓库。

## 11. V2 天空条纹审查与修正（2026-10-08）

**域偏差成立，但编码方向须纠正。** scanner 提供 linear RGB；纯 gamma 的编码是 `linear^(1/2.2)`，解码才是 `encoded^2.2`。V2 现在在内部使用固定 Rec.709 分段 transfer，将 Film Resolution、亮度分区、双场亮度混合和 Overlay 放入显示编码域，再返回 linear，由 scanner 完成目的输出编码。固定 transfer 是明确的兼容选择；不将辅助函数的存在解释成 Photo `by_pass` 强制使用该 transfer。目的色域 primaries 未转换，非 Rec.709 输出仍属于兼容实现。分段线性趾保留负值，未额外裁掉扩展范围。

**合成天空的波纹与旧 clustered 旋转有关。** 原 V2 用低频 `pnoise(pos*8)` 改变绕整幅中心旋转的角度，使采样坐标形成局部方向性脊纹。1024×576 渐变天空、35mm250、seed=42 的隔离实验中，关闭 clustered 后脊纹消失；修正为按虚拟纹理 texel 的整数 hash 产生旋转角，同时按原 kernel 的顺序先对 pos 除 generator scale，再旋转和乘 canvas/cluster。该方向 hash 保留 CPU/GPU 的可重复性，不声称复刻原浮点位 hash。

**Noise 的位置种子偏差成立。** 原 kernel 用 `snoise(timer, inColor)`，输入包含 RGBA；RGB-only 框架的 alpha 是常量，不能用 luma 代替 alpha，也不能把 XY 混入相位。V2 使用 RGB-only 连续整数梯度场构造相位，保证相同 RGB 的相位不依赖位置，并对同亮度异色输入作区分。这修正依赖关系，但不是原 `snoise` 或 sine permutation texture 的数值复刻；原 Noise 的 `cluster/256` 与 Analogue 的 `1/(256*cluster)` 仍是独立哈希方案的已知兼容边界。

Analogue 仍对九个重采样 tap 共用目标像素亮度；原生成器在每个虚拟 texel 读取源图亮度。该差异可能影响强边缘附近的颗粒，属于尚未逐值复刻的边界，与本次低频旋转导致的天空脊纹分开记录。

**Noise 的 Film Resolution 语义已纠正。** `ScanGrainKernel` 直接从输入图像生成 Noise；SpektraFilm 的 V2 Noise 路径因此将 `resolution_radius` 固定为 0，不再使用历史 `1.87` 半径或 `effective(amount)×0.5` 强度。Noise 的 scale 仍按 `(1+(s−1)/47) * 2.4 * max(W/1920,H/1080)` 计算。

**零值遵循参数直通语义。** `Amount=0` 或三个亮度分区全部为 0 时，SpektraFilm V2 返回输入图像；单个分区强度直接使用 0–1 值，不经过额外 `effective_control()` 曲线。

**GPU 路由边界。** `resolution_type=1` 的 FastBlur 路径有 CPU/WGPU 对照；`resolution_type=0` 的 OpticalResolution 保留 CPU 兼容实现，WGPU 路由回退 CPU。当前内置 profile 均使用 FastBlur。

**GSF 已实现，不是漏掉固定画布。** 实际虚拟尺寸为 `image*max(5200/W,3100/H)`，保持宽高比。`−0.2` 取整微调未复刻；按需程序采样不分配画布纹理，因此不施加某张显卡的 texture cap。封顶在极端宽高比下可能远大于 1px，不能一概称为微小项。当前 photo 管线无 overscan/damage mask 输入；未引入虚假的 mask 接线。

**验证范围。** 1024×576 天空局部横纵相关性差 RMS：旧代码 `0.15443`，修正后 `0.04676`，回归门槛 `0.10`；同一临时 runner 对旧/新模块实测。最终 f64 Grain V2 测试 8 项通过，含中灰分区、同亮度异色 Noise 相位、Noise 边缘/尺度、12 profiles × 2 modes CPU/GPU 对照。768×512 f64 CLI 32-bit TIFF 开启/关闭对照以 ffmpeg 解码，去行均值残差 RMS 为 Analogue `0.00401`、最终 Noise `0.00799`；纸基扫描 16-bit TIFF 也完成导出及读取。6000×4000 合成天空与额外 GPU smoke 是宿主 Noise 映射修正前的证据，不作为最终 Noise 数值验收。这些均为合成输入，不构成用户原始照片已修复的证明。

## 12. 公开参数对齐（2026-10-08）

- 对外保留 Grain Profiles / Custom、Film Type、Processing Mode、Size、Amount、Shadows、Midtones、Highlights、Film Resolution、Chroma、Enabled。移除独立 Resolution filter、V2 timer 和额外 reset 行为。静态相位复用 recipe `random_seed`。
- Amount、三个亮度分区和 Chroma 的 GUI/JSON 范围均为 0–100，运行时除以 100；Size 为 1–48，Film Resolution 为 0–100。
- 预设下 Amount 仍可调：构造函数 `0x57cb9a–0x57cba3` 将 grainAmount 存入 context+`0x28`，没有加入隐藏控件列表；`update_state` 在 `0x57e250–0x57e264` 复制预设后，`0x57e280–0x57e292` 再读取并覆盖 Amount。其他控件只在 Custom 下生效。GUI 切换 Custom 复制当前有效参数，选择新预设恢复其 Amount。
- Film Type 的执行分支已确认：`FilmGrainKernel::process` 在 `0x5ed417` 比较复制后的 state+`0x9ac`；Negative=0 跳到 `0x5ed4f8` 并调用 OpticalResolution（`0x5ed611`），Positive=1 调用 FastBlur（`0x5ed471`）。内置预设的 resolution type 均为 1，因此保持 Positive 分支。SpektraFilm 使用既有 Gaussian / fractional box FIR 兼容实现，未宣称还原参考 PSF。
- 本次验收：f32/f64 各 9 项 grain 测试通过，包含 Film Type 边缘响应、零 Amount 非旁路和 CPU/WGPU 对照；shader device check、预设/Custom 参数继承及 4 项 GUI state 测试通过。f32/f64 各完成 10 个 TIFF render/export 往返（V1 对照及 V2 两种 Film Type × 两种模式，均覆盖 film/paper 输出）。
- 独立 release f64 CLI 的 768×512 合成天空导出：Negative/Positive 最大像素差为 Analogue `0.0000915825`、Noise `0.0012207031`；Amount 与三个分区全零相对显式禁用的 RMS 为 `0.0021044274`。这些数值证明当前实现的分支与零端点有效，不证明与 Dehancer 像素一致。
- 原生 GUI 实际完成 Positive/Noise WGPU 扫描；选 `8mm500` 后只显示 Amount，修改至 25 再切 Custom，继承 Positive/Analogue、Size 48、Shadows 30、Midtones 45、Highlights 65、Film Resolution 75、Chroma 90、Amount 25。当前隔离显示环境的 Save state 未出现文件对话框，因此未计入本次原生保存验收；状态持久化由上述 roundtrip 测试覆盖。
- 历史天空数值保留于上一节；本次结果仍受独立 hash、Noise RGB-only 相位及 Gaussian/fractional-box PSF 兼容实现的限制。硬件 render node 权限不足，WGPU 证据不代表独显性能。

## 13. 原始内核对照进行中（2026-10-09）

- 原始 OpenCL 内核在本机 PoCL 上运行；37×19 合成输入、8mm50 参数、固定诊断相位 `42/65536`、Film Resolution=100、6036×3100 RGBA half 颗粒纹理下，当前 CPU 相对原始输出最大误差为 `0.066584587`、RMS 为 `0.007993329`。固定相位用于隔离算法，并非已证实的宿主 seed 映射。此结果未达到完整对齐。
- WGPU 24 / Mesa 26.0.8 llvmpipe 创建完整颗粒管线曾报告 device lost；在相同完整 shader 中将大角度归约循环内的常量数组索引改为标量选择后，实际渲染成功。独立最小常量数组索引能编译，因此该现象只定位到此归约上下文，不能泛化为所有动态数组索引故障。归约提取窗口已用 7,590 组输入对照精确整数乘积，覆盖全部大角度有限 f32 指数。
- 编译修复后的首次 CPU/WGPU 渲染最大误差为 `0.1404953`。继续拆分小角度归约常量、统一普通乘加，并在 shader 的 sine-hash 中用精确 `frexp/ldexp` 固定乘法舍入边界后，同一 Analogue 对照降至 `0.000033676624`；64 组诊断输入的 13 个噪声中间量逐位一致。跨 1,139 个角度的三角函数诊断仍有 30 行出现最多 `5.9604645e-8` 的后端差异。
- 更新后的 Analogue 相对原始 half 纹理输出：CPU 最大误差 `0.050306439`、RMS `0.007752230`；WGPU 最大误差 `0.050290763`、RMS `0.007752125`。同一输入的 Noise CPU/WGPU 最大误差仍为 `0.1786393`。原始参考差异与完整后端矩阵均未通过；不得沿用上一节的内部一致性验收作为当前实现的完成证据。

## 14. 独立工作树的参考闭环（2026-10-09）

本节替代第 11–13 节对当前代码的描述；旧数值仅记录调查过程。实现位于 `/data/worktrees/spektrafilm-grain-v2-reference-integration`，分支 `work/grain-v2-reference-integration`。没有修改主工作区，也没有引入原版二进制或专有 kernel 运行时依赖。

验收标准已由用户明确：要求算法、处理逻辑和视觉效果对齐，不要求每一个像素一致。因此厂商 sin/cos 的浮点差异本身不是阻塞项；仍需排除工作色域、参数映射、采样和滤波顺序差异，以及颗粒强度、尺度、频谱或色彩相关性的系统性偏差。以下原版逐像素误差仅作诊断记录，不单独作为效果不一致的证据。


### 已恢复的执行语义

- 静态 seed 经 MT19937 和两次取数的 `generate_canonical<double>` 运算转为 float 相位，替代未经证实的 `seed/65536`。seed 5489 的相位位模式为 `0x3e0aba7c`。原版宿主的全局随机流不等于 SpektraFilm 的可重复 recipe seed；后者是现有产品契约。
- 工作 RGB、生成颗粒和合成输出按 half 存储边界舍入。WGSL 使用显式 IEEE round-to-nearest-even，避免驱动将 pack/unpack 往返消除。Optical 的 H/V 之间保持 float；FastBlur 的每一遍均写 half。
- Negative 使用 Optical 平顶核；Positive 使用折叠 Gaussian FastBlur，保留原 line kernel 的越界回到中心和采样偏移。Analogue 生成器读取未做 Film Resolution 的原始工作图，合成读取滤波结果；Noise 的相位、噪声与合成都读取滤波结果。
- Noise 使用 RGBA 内容相位（alpha=1），Analogue 逐个虚拟 texel 读取源图。两种模式恢复 sine permutation 和三维梯度插值；共享的独立三角函数、乘法舍入边界使 CPU/WGSL 可重复。
- 虚拟纹理尺寸为 `floor(image_size * max(5200/W,3100/H))`，原宿主没有旧报告所称的 `−0.2` 微调。按需生成不分配该纹理，因此不模拟设备的纹理尺寸上限。

### 三类对照分别计量

固定输入为 37×19 RGB，行优先通道值 `(index % 17)/17`；profile=8mm50，seed=5489，Film Resolution=100。原版内核从外部研究目录运行时读取，未复制进产品。

| 对照 | Analogue 最大绝对差 | Noise 最大绝对差 | 含义 |
|---|---:|---:|---|
| 产品 CPU/WGSL | 2.38e-7 | 2.98e-7 | 本机 llvmpipe 的内部一致性 |
| 产品 CPU / 未修改原版 OpenCL（PoCL） | 0.05559057 | 0.16422206 | **没有达到原版逐像素一致** |
| 产品 CPU / 数值归一化的原版执行 | 5.96e-8 | 5.96e-8 | 隔离算法拓扑与数值执行差异；不是原版原样执行 |

数值归一化仅用于外部诊断：替换 `rnm` / `coord_rot` 的 sin/cos 为独立的共享实现，显式标量相位归约，并关闭 OpenCL contraction。宿主参数按逐步 binary32 计算；Python 双精度后一次 cast 会改变 Noise scale，曾导致 12 个通道样本出现最多 0.00077087 残差。修正诊断脚本后该残差消失。

首个分歧的独立证据：8,436 次 rnm 调用的正弦输入逐位一致，独立 sin 与 PoCL 的 736 个结果不同，最大仅 5.96e-8；乘 43758.5453 并取 fract 后，差异可达约 1.99。换成系统 libm 也有 1,737 个不同结果。原版相位 dot 另有 41/703 个像素最多 5.96e-8 的差异。归一化后，703 像素的 RGB、snoise、相位、旋转坐标和三通道 pnoise 全部逐位一致。

完整的 2,109 通道 half 输出分别保存在 `crates/spektrafilm-model/src/grain/fixtures/{analogue,noise}_normalized_half.txt`。`matches_normalized_external_kernel_outputs` 在 f32/f64 均通过，比较所有输出，不以统计分布替代逐样本检查。fixture 是归一化外部执行的结果数据，不含专有源码，不得标为原版逐字节 fixture。

Film Resolution 另用 17×11 half 渐变加边缘输入，覆盖两类滤波 × 半径 0.1/0.5/1/2/4 × H/V 共 20 个阶段。实际 Rust 函数和实际 WGSL 分别执行：最终 half 输出在 10 个案例中均与原版一致；Optical float H 中间结果最大差为 Rust 1.19e-7、WGSL 1.79e-7。诊断脚本 `/tmp/grain_actual_filter_comparison.py`，结果 `/tmp/grain-actual-filter-comparison.json`。

### 原生 GUI 与导出

独立 Xvfb + `dbus-run-session` 下真实启动 GUI，完成 WGPU Analogue/Noise Scan；保存并加载 V2 Custom/Negative/Noise、Amount=35 的 JSON 状态。此前不弹保存框是显示与会话总线环境问题；本次实际 portal 对话框成功写出文件。

GUI 的 Save、CPU f64 Export（Analogue）、加载 Noise 状态后的 Export 均产出可由 ffmpeg 解码的 768×512 RGB 16-bit TIFF。两种模式导出差 RMS=0.02763659，只证明模式切换和输出链路有效。文件为 `/tmp/grain-integration-native-save.tif`、`/tmp/grain-integration-native-export.tif`、`/tmp/grain-integration-native-noise-export.tif`。已有 f32/f64 render/export 矩阵包含 V1 和 V2 两种 Film Type、两种模式、film/paper 输出；V1 默认与公开参数未改变。

### 尚不能宣称的结果

没有实际 Dehancer vendor-device 阶段 dump；PoCL 是原始 OpenCL 源码的一个执行实现，不代表所有厂商 GPU 的数值 ABI。未经归一化的原版输出差异仍然存在，不能以内部一致性或归一化对照宣布“完整像素对齐”。也尚未验证宿主 FastBlur 超限半径的 downscale Options 链、极端宽高比纹理 cap、照片版 PE 宿主与 OFX 的差异。本机硬件 render node 无权限，性能证据仅来自 llvmpipe。

对于结构化 CLI render 的 `maxEdge=9568`、Size≤48、Amount≤100，FastBlur 半径上界为 `2×9568/5200×1.87=6.8816`，低于原宿主触发 downscale 的 `17√2≈24.0416`；这条受限入口不会走未恢复的超限路径。该结论不外推到无此尺寸限制的直接模型调用或额外 upscale。


### 参考数据溯源与退出验证

- 外部原始 kernel SHA-256：`0ed87bcc78a78bfc5d46a74c20cd290d9dbfe27e88f0ce0f50daa4a6d7472ae5`。
- 归一化 runner `/tmp/grain_normalized_f32_reference.py` SHA-256：`1d0b5a54828e183a0587cd34de39e7daf74c5d77160b25ee10763d509f199c0d`；独立三角函数 `/tmp/grain_compat_trig.cl`：`a0474e0ceb1956a152b0fda8dfe2f4d618e965927352274541bfd8fe461541fc`。
- Analogue fixture SHA-256：`2ca2e3802ccfdecb065480a5c3cb897a7cce3e7ff84c4224d34895f06240d840`；Noise fixture：`88dfa5ee26cc16eefe8e5a83607b3b932f0e80bf9ab4cb6d047a624d69194888`。
- 验收结束时 `xdotool windowclose` 销毁 X11 窗口，winit 0.30.13 随后的 `TranslateCoordinates` 请求触发 BadWindow panic，GUI 返回 101。重新启动加载保存的 Noise 状态，经窗口管理器 Alt+F4 关闭，GUI 返回 0；xdotool 的后续按键释放因窗口已退出报告 BadWindow。此处区分自动化命令结果与应用退出结果；导出文件在关闭前已独立解码验证。


### 按算法与效果验收补查

Negative 分支的 OpticalResolution 边界实参已确认：OFX `FilmGrainKernel::process` 在 `0x5ed60c` 执行 `xor ecx,ecx`，随后 `0x5ed611` 调用 `OpticalResolution(void*, float, ChannelsDesc::Transform const&, DHCR_EdgeMode, bool, string const&)`。SysV ABI 的枚举实参在 ECX，值为 0；外部 kernel 的 `DHCR_EdgeMode` 顺序为 CLAMP=0、BORDER=1、WRAP=2。当前 CLAMP 实现符合宿主调用。

工作域疑点已由后续宿主报告 §6.1–6.5 解除，当前实现与验证见第 15 节。此前固定 BT.709 包装已删除。

Noise 的输入绑定已直接核实：`ScanGrainKernel::process` 在 `0x5eeb67` / `0x5eed04` 将源图滤波到 destination；执行回调 `0x5ef280` 在 `0x5ef29f`、`0x5ef2b6` 两次通过 vtable+0x70 取 destination，分别绑定 kernel arg0 和 arg1（`0x5ef2ad`、`0x5ef2c7`）。因此原版 Noise 从 Film Resolution 结果计算颜色相位，当前 CPU/WGSL 一致；不能把 Analogue 生成器的原图输入约定套用到 Noise。原版 `blend_normal` 在外部源码 `1446–1461` 明确 clamp 到 [0,1]，当前最终合成 clamp 也符合该逻辑。


### 较大样本的效果对照

实际运行 `/tmp/grain_effect_acceptance.py`：384×256 分区明暗、彩色渐变及棋盘细节，8mm50、seed=5489、Film Resolution=100（滤波半径为零），两种模式。外部原版 OpenCL 使用编码后 half RGB 输入，输出解码回线性后与产品比较；产品输入为原始线性 RGB。残差基线为输入经过相同 encode/half/decode 的结果。此前直接混比编码域与线性域的统计无效，以下为修正后结果。

| 模式 | 原版残差均值 | CPU 残差均值 | 原版残差标准差 | CPU 残差标准差 |
|---|---:|---:|---:|---:|
| Analogue | -0.03418593 | -0.03421378 | 0.04269066 | 0.04271103 |
| Noise | -0.01951116 | -0.01949283 | 0.03927353 | 0.03929670 |

标准差相对差分别约 0.048% 和 0.059%；各明暗区域标准差相对差低于 0.4%。相邻像素相关系数最大绝对差约 0.00534；去均值、正确折返 FFT 频率轴后的频带幅值，最大相对差约 3.56%（Noise 低频段）。已查看包含输入、两种模式的原版/CPU/WGPU 的七列对照图。这组样本支持颗粒强度与空间分布接近，不把随机实现之间的逐像素 RMS 当作效果失败。

较大样本也修正内部一致性的范围：CPU/WGPU 最大差 Analogue=0.00095546、Noise=0.121766，RMS 分别约 7.969e-6、0.00060345；Noise 有 50/294912 个通道差超过 0.01。较小样本上 `<3e-7` 的结果不能泛化为所有输入的逐像素保证。这些稀疏差异尚未逐阶段归因；当前验收允许浮点导致的随机纹理差异，但仍应保留该证据。

历史产物：`/tmp/grain-effect-acceptance.json`、`/tmp/grain-effect-input.bin`、`/tmp/grain-effect-input-encoded.bin`。该次统计仅覆盖一个图像、profile 和 seed、零滤波半径，当时工作域边界尚待确认；后续证据、实现修正和重新执行结果见第 15 节。`/tmp/grain-effect-panel.png` 已由新接口对照更新。

## 15. 原生显示编码工作域闭环（2026-10-09）

新增外部证据：`dehancer-re` 提交 `cbec289`（转换链与域结论）、`e0dcc88`（DVRWGRec709 为保灰非线性 gamut 重映射，非矩阵）、`fe7622b`（22 档灰阶表）。宿主报告 §6.1–6.5 证明照片 by_pass 链不在 Grain 内施加传递函数或 primaries 转换。Grain 约定显示编码 RGB，曲线来自调用者；不把视频 ocio LUT 加到照片颗粒链。

CPU `apply_cpu`、WGPU `grain_v2_gpu` 已统一为原生编码 RGB 输入/输出，保留 half 存储边界，删除固定 BT.709 编解码。SpektraFilm 扫描产生新的 RGB 图像，故调用层使用所选扫描输出空间（不是原照片输入空间）的编码；encoded 输出直接保留 Grain 结果，linear 输出只解码所选曲线。已有 same-space 矩阵往返仅执行一次。V1 的路径与默认值保持原行为。

本轮实跑 `scripts/parity/grain_v2_acceptance.py` 全部通过：f32/f64 各 9 个模型测试（含原外部完整 half fixture、12 profiles 的 CPU/WGPU 分派）、WGSL 编译、参数继承；新增扫描域回归在两种精度下覆盖 sRGB、Display P3、ProPhoto RGB、ITU-R BT.2020 × 胶片/纸基扫描 × Analogue/Noise，验证 Grain 消费已编码扫描结果和 linear/encoded 导出的一致性。真实 pipeline render/save/load 完成 f32/f64 各 10 组 32-bit TIFF 输出，包含 V1 与 V2 两种模式和 Film Type。没有将历史 GUI 实测冒充本轮重新运行。

外部 22 档灰阶表按原 3/64 间距复核，sRGB/Rec709/gamma2.2 三列最大误差均小于 5e-7，符合 CSV 六位小数舍入。独立 OpenCL 对照在 384×256、8mm50、seed5489、Film Resolution=100 的同一已编码 half 输入上重新执行；CPU/WGPU 编码域两种模式最大差均为 0.00048828125。用于统计的解码线性域 RMS 分别为 5.469e-6、5.438e-6，没有通道差超过 0.01；此前 Noise 大离群值在该样本上消失。原版/CPU 残差标准差分别为 Analogue 0.04269066/0.04271103，Noise 0.03927353/0.03929687。产物 `/tmp/grain-native-domain-acceptance.json`。

验收以算法、处理逻辑和效果为准，不要求厂商设备逐像素相同。效果统计限于上述样本；现有独立滤波证据及控制矩阵覆盖其余实现路径，不声称测遍任意照片。本节记录独立 integration 工作树完成域修正时的验证；后续主干集成另见下节。

## 16. 主干驻留链集成（2026-10-09）

集成远端 `5b538ed` 的模块拆分、统一 GPU 参数与 pipeline cache。独立 WGPU 调用和驻留链共用输入 half 舍入、可选水平/垂直 Film Resolution、颗粒合成三段调度；驻留链在 GPU 上先执行扫描输出空间的 same-space 矩阵和 CCTF。V2 输出保持编码 RGB，线性导出只解码一次。恢复 Film Type 选择滤波分支、seeded phase 和参考半精度边界。

合并后 `scripts/parity/grain_v2_acceptance.py` 通过：f32/f64 各 9 个模型测试、WGSL 编译、profile 参数继承、两种精度下扫描编码回归，以及每种精度 10 组真实 TIFF render/save/load。`cargo check --workspace --all-targets --all-features` 与 GUI 的 2 个 Grain V2 状态测试通过；已有 unused/dead-code 等编译警告仍存在。

临时实际 pipeline smoke 使用 32×24 非均匀 RGB，覆盖 8 个注册输出空间 × 胶片/纸基扫描 × Analogue/Noise × Negative/Positive，共 64 组；分别在默认分辨率参数和 Size=48、Film Resolution=0 下运行。驻留路径明确禁止 fallback，输出与同一无颗粒驻留基底上的独立 GPU Grain 完全一致，线性导出相对所选 CCTF 解码最大误差为 `5.876e-8`。这是本机可用 WGPU adapter 的执行证据，不是独立显卡性能证明；本轮未重跑原生 GUI 交互。临时 smoke 源已删除。

