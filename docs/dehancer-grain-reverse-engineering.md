# Dehancer 颗粒渲染逆向分析

> 目标：下载 Dehancer 官方发行版，对其程序做静态逆向，还原其 Film Grain（颗粒）渲染的完整实现，作为 SpektraFilm 颗粒模块设计与学习的参考。
>
> 分析对象：`DehancerPro_InstallerDavinciResolve_Linux_7.4.1.tar.gz`（官方 CDN 直链下载，2026-06-30 构建）
> 内含 OFX 插件 `DehancerProOpenCL_x86_64_v7.ofx`（161 MB，stripped ELF x86-64，内嵌 OpenCL kernel 源码与 cereal JSON 配置）。
>
> 本文档只记录算法行为、参数语义与公式（以自有伪代码/公式表述），不含 Dehancer 源码原文。

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
- `Film Type` 不再选择滤波路径；`resolution_type` 单独控制 FastBlur/OpticalResolution。内置 profile 的 `resolution_type=1`，Custom 继承该值。
- 本次验证：`cargo test -p spektrafilm-model --no-default-features --lib` 通过 40 项；`cargo test -p spektrafilm-core` 通过 138 项；`grain_v2_acceptance` 完成 10 个 TIFF render/export 往返，覆盖 V2 Analogue/Noise、Negative/Positive UI 状态及 film/paper scan。
- FastBlur 边界单测锁定半径 0.5 的 Gaussian folded weight/offset；Noise 的 `resolution_factor` 不改变输出；Amount=0 为 identity；CPU/WGPU 对照覆盖 12 profiles、FastBlur、Analogue/Noise。
- 这些结果证明当前 Rust 路由、参数边界和导出链可运行，不证明与 Dehancer 输出逐像素一致；OpticalResolution 的 GPU 路由仍明确回退 CPU。
