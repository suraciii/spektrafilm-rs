use super::native;

#[derive(Debug)]
pub(crate) struct Invocation {
    pub configuration: Vec<f32>,
    pub exposure: Vec<f32>,
    pub film: Vec<f32>,
    pub paper: Option<Vec<f32>>,
    pub mask: i32,
    pub seed: u32,
    pub dimension: i32,
    pub support: i32,
}
#[derive(Debug)]
pub(crate) struct Component {
    pub head: Invocation,
    pub stencils: Vec<f32>,
}
#[derive(Debug)]
pub(crate) struct Prepared {
    pub width: u32,
    pub height: u32,
    pub base: Invocation,
    pub tiles: Vec<f32>,
    pub tail: Option<Invocation>,
    pub components: Vec<Component>,
    pub records: i32,
}
impl Prepared {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = Cursor { bytes, offset: 0 };
        if cursor.take(4)? != b"FSA1" {
            return Err("Unknown Fotufilm prepared frame format".into());
        }
        if cursor.u32()? != 2 {
            return Err("Unsupported Fotufilm prepared frame version".into());
        }
        let width = cursor.u32()?;
        let height = cursor.u32()?;
        if width == 0 || height == 0 {
            return Err("Fotufilm prepared frame has empty dimensions".into());
        }
        let mask = cursor.u32()? as i32;
        let seed = cursor.u32()?;
        let configuration_count = cursor.u32()? as usize;
        let dimension = cursor.u32()?;
        let lut_count = cursor.u32()? as usize;
        let paper_present = cursor.u32()?;
        let tile_count = cursor.u32()? as usize;
        let component_count = cursor.u32()? as usize;
        let support = cursor.u32()?;
        let records = cursor.u32()?;
        let plan_kind = cursor.u32()?;
        if plan_kind > 1 || (plan_kind == 0 && component_count != 0) {
            return Err("Fotufilm prepared frame has an invalid transport plan".into());
        }
        let (config_expected, lut_expected, tiles_expected) = native::counts();
        if configuration_count != config_expected
            || lut_count != lut_expected
            || dimension != 33
            || paper_present > 1
            || !matches!(records, 3 | 4)
            || support > i32::MAX as u32
            || component_count > 24
            || (tile_count != 0 && tile_count != tiles_expected)
        {
            return Err(
                "Fotufilm prepared frame is incompatible with the pinned native graph".into(),
            );
        }
        let configuration = cursor.floats(configuration_count)?;
        let exposure = cursor.floats(lut_count)?;
        let film = cursor.floats(lut_count)?;
        let paper = if paper_present == 1 {
            Some(cursor.floats(lut_count)?)
        } else {
            None
        };
        let tiles = cursor.floats(tile_count)?;
        let base = Invocation {
            configuration,
            exposure,
            film,
            paper,
            mask,
            seed,
            dimension: dimension as i32,
            support: support as i32,
        };
        let mut components = Vec::with_capacity(component_count);
        let tail = if plan_kind == 1 {
            let mask = cursor.u32()? as i32;
            let seed = cursor.u32()?;
            let configuration = cursor.floats(configuration_count)?;
            Some(Invocation {
                configuration,
                exposure: Vec::new(),
                film: Vec::new(),
                paper: None,
                mask,
                seed,
                dimension: dimension as i32,
                support: support as i32,
            })
        } else {
            None
        };
        for _ in 0..component_count {
            let mask = cursor.u32()? as i32;
            let seed = cursor.u32()?;
            let exposure_count = cursor.u32()? as usize;
            let stencil_count = cursor.u32()? as usize;
            if exposure_count != lut_count || stencil_count == 0 {
                return Err("Fotufilm transport component has invalid tables".into());
            }
            let configuration = cursor.floats(configuration_count)?;
            let exposure = cursor.floats(exposure_count)?;
            let stencils = cursor.floats(stencil_count)?;
            components.push(Component {
                head: Invocation {
                    configuration,
                    exposure,
                    film: Vec::new(),
                    paper: None,
                    mask,
                    seed,
                    dimension: dimension as i32,
                    support: support as i32,
                },
                stencils,
            });
        }
        if cursor.offset != bytes.len() {
            return Err("Fotufilm prepared frame contains an unrecognized trailing section".into());
        }
        Ok(Self {
            width,
            height,
            base,
            tiles,
            tail,
            components,
            records: records as i32,
        })
    }
}
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or("Fotufilm prepared section length overflow")?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or("Fotufilm prepared section is truncated")?;
        self.offset = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn floats(&mut self, count: usize) -> Result<Vec<f32>, String> {
        let bytes = self.take(
            count
                .checked_mul(4)
                .ok_or("Fotufilm float section length overflow")?,
        )?;
        let values: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect();
        if values.iter().any(|v| !v.is_finite()) {
            return Err("Fotufilm prepared arrays contain nonfinite samples".into());
        }
        Ok(values)
    }
}
