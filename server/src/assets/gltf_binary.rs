//! Streaming scalar checks: no second BIN allocation and no temporary SSD spool.
use super::{ApiError,ApiResult};
use serde_json::Value;
use std::{collections::BTreeMap,io::{self,Read}};

#[derive(Clone,Copy)]
enum Rule { Float, TimeBase(usize), TimeSparse(usize), TimeIndex(usize,u64), Index(u64), SparseIndex(u64) }
struct TimeSeries { values: Vec<f32>, base_next: usize, indices: Vec<u32>, sparse: Vec<f32> }
struct Scalar {
    offset: u64,
    stride: u64,
    remaining: u64,
    width: usize,
    rule: Rule,
    partial: [u8;4],
    used: usize,
    previous: Option<f64>,
}
impl Scalar {
    fn observe(&mut self, start: u64, bytes: &[u8], times: &mut [TimeSeries]) -> io::Result<()> {
        let end=start+bytes.len() as u64;
        while self.remaining > 0 {
            let next=self.offset+self.used as u64;
            if next >= end { break; }
            if next < start { return Err(io::Error::new(io::ErrorKind::InvalidData,"Noncontiguous GLB scalar stream")); }
            let count=(self.width-self.used).min((end-next) as usize);
            let source=(next-start) as usize;
            self.partial[self.used..self.used+count].copy_from_slice(&bytes[source..source+count]);
            self.used+=count;
            if self.used != self.width { break; }
            let value=match self.rule {
                Rule::Float|Rule::TimeBase(_)|Rule::TimeSparse(_)=>f32::from_le_bytes(self.partial) as f64,
                Rule::Index(_)|Rule::SparseIndex(_)|Rule::TimeIndex(_,_)=>match self.width {
                    1=>self.partial[0] as f64,
                    2=>u16::from_le_bytes(self.partial[..2].try_into().unwrap()) as f64,
                    _=>u32::from_le_bytes(self.partial) as f64,
                },
            };
            let valid=match self.rule {
                Rule::Float=>value.is_finite() && value.abs()<=100_000_000.0,
                Rule::TimeBase(_)|Rule::TimeSparse(_)=>value.is_finite() && (0.0..=86400.0).contains(&value),
                Rule::Index(limit)=>value < limit as f64,
                Rule::SparseIndex(limit)|Rule::TimeIndex(_,limit)=>value < limit as f64 && self.previous.is_none_or(|previous|value>previous),
            };
            if !valid { return Err(io::Error::new(io::ErrorKind::InvalidData,"Nonfinite/out-of-range or unordered GLB scalar")); }
            match self.rule {
                Rule::TimeBase(slot)=>{
                    let time=&mut times[slot];
                    time.values[time.base_next]=value as f32;
                    time.base_next+=1;
                },
                Rule::TimeSparse(slot)=>times[slot].sparse.push(value as f32),
                Rule::TimeIndex(slot,_)=>times[slot].indices.push(value as u32),
                _=>{},
            }
            self.previous=Some(value);
            self.remaining-=1;
            self.offset+=self.stride;
            self.used=0;
        }
        Ok(())
    }
}
pub struct Checked<R> { reader: R, pending: Vec<Scalar>, active: Vec<Scalar>, position: u64, times: Vec<TimeSeries> }
impl<R:Read> Checked<R> {
    pub fn new(reader: R, doc: &Value) -> ApiResult<Self> {
        let mut bounds=BTreeMap::<usize,u64>::new();
        let accessors=doc["accessors"].as_array().map(Vec::as_slice).unwrap_or_default();
        let meshes=doc["meshes"].as_array().map(Vec::as_slice).unwrap_or_default();
        let mut times=std::collections::HashSet::new();
        if let Some(animations)=doc["animations"].as_array() {
            for animation in animations {
                if let Some(samplers)=animation["samplers"].as_array() {
                    for sampler in samplers { if let Some(input)=sampler["input"].as_u64() {times.insert(input as usize);} }
                }
            }
        }
        for mesh in meshes {
            if let Some(primitives)=mesh["primitives"].as_array() {
                for primitive in primitives {
                    if let Some(reference)=primitive["indices"].as_u64() {
                        let position=primitive["attributes"]["POSITION"].as_u64().unwrap() as usize;
                        let count=accessors[position]["count"].as_u64().unwrap();
                        bounds.entry(reference as usize).and_modify(|bound|*bound=(*bound).min(count)).or_insert(count);
                    }
                }
            }
        }
        if let Some(nodes)=doc["nodes"].as_array() {
            for node in nodes {
                if let (Some(mesh),Some(skin))=(node["mesh"].as_u64(),node["skin"].as_u64()) {
                    let joints=doc["skins"][skin as usize]["joints"].as_array().unwrap().len() as u64;
                    for primitive in meshes[mesh as usize]["primitives"].as_array().unwrap() {
                        for name in ["JOINTS_0","JOINTS_1"] {
                            if let Some(reference)=primitive["attributes"][name].as_u64() {
                                bounds.entry(reference as usize).and_modify(|bound|*bound=(*bound).min(joints)).or_insert(joints);
                            }
                        }
                    }
                }
            }
        }
        let mut time_series=Vec::new();
        let mut time_slots=BTreeMap::new();
        let mut time_count=0usize;
        for index in &times {
            let count=accessors[*index]["count"].as_u64().unwrap() as usize;
            time_count=time_count.checked_add(count).filter(|n|*n<=262144)
                .ok_or_else(||ApiError::bad("GLB animation time buffer budget exceeded"))?;
            time_slots.insert(*index,time_series.len());
            time_series.push(TimeSeries {values:vec![0.0;count],base_next:0,indices:Vec::new(),sparse:Vec::new()});
        }
        let mut pending=Vec::new();
        for (index,accessor) in accessors.iter().enumerate() {
            let component=accessor["componentType"].as_u64().unwrap();
            let width=match component {5120|5121=>1,5122|5123=>2,_=>4};
            let (rows,columns)=match accessor["type"].as_str().unwrap() {
                "SCALAR"=>(1,1),"VEC2"=>(2,1),"VEC3"=>(3,1),"VEC4"=>(4,1),
                "MAT2"=>(2,2),"MAT3"=>(3,3),"MAT4"=>(4,4),_=>unreachable!(),
            };
            let column=if columns>1 {(rows*width).div_ceil(4)*4} else {rows*width};
            let element=column*columns;
            let rule=if let Some(slot)=time_slots.get(&index) {Some(Rule::TimeBase(*slot))} else if component==5126 {Some(Rule::Float)}
                else {bounds.get(&index).copied().map(Rule::Index)};
            let count=accessor["count"].as_u64().unwrap();
            if let Some(rule)=rule
                && let Some(view)=accessor["bufferView"].as_u64() {
                add(&mut pending,doc,view,accessor.get("byteOffset").and_then(Value::as_u64).unwrap_or(0),
                    count,width,rows,columns,column,element,rule,true)?;
            }
            if let Some(sparse)=accessor.get("sparse") {
                let count=sparse["count"].as_u64().unwrap();
                let indices=&sparse["indices"];
                let index_width=match indices["componentType"].as_u64().unwrap() {5121=>1,5123=>2,_=>4};
                let limit=accessor["count"].as_u64().unwrap();
                let index_rule=if let Some(slot)=time_slots.get(&index) {Rule::TimeIndex(*slot,limit)} else {Rule::SparseIndex(limit)};
                add(&mut pending,doc,indices["bufferView"].as_u64().unwrap(),indices.get("byteOffset").and_then(Value::as_u64).unwrap_or(0),
                    count,index_width,1,1,index_width,index_width,index_rule,false)?;
                if let Some(rule)=rule {
                    let values=&sparse["values"];
                    let rule=if let Rule::TimeBase(slot)=rule {Rule::TimeSparse(slot)} else {rule};
                    add(&mut pending,doc,values["bufferView"].as_u64().unwrap(),values.get("byteOffset").and_then(Value::as_u64).unwrap_or(0),
                        count,width,rows,columns,column,element,rule,false)?;
                }
            }
        }
        pending.sort_by_key(|scalar|std::cmp::Reverse(scalar.offset));
        Ok(Self {reader,pending,active:Vec::new(),position:0,times:time_series})
    }
    pub fn position(&self) -> u64 { self.position }
    pub fn finish(&mut self) -> ApiResult<()> {
        if !self.pending.is_empty() || self.active.iter().any(|scalar|scalar.remaining>0) {
            return Err(ApiError::bad("Truncated GLB scalar data"));
        }
        for time in &mut self.times {
            if time.indices.len()!=time.sparse.len() { return Err(ApiError::bad("Truncated sparse animation times")); }
            for (index,value) in time.indices.iter().zip(&time.sparse) { time.values[*index as usize]=*value; }
            if time.values.windows(2).any(|pair|pair[0]>=pair[1]) {
                return Err(ApiError::bad("Animation times must increase strictly"));
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn add(output: &mut Vec<Scalar>, doc: &Value, view: u64, local: u64, count: u64,
    width: u64, rows: u64, columns: u64, column: u64, element: u64, rule: Rule, interleaved: bool) -> ApiResult<()> {
    let view=&doc["bufferViews"][view as usize];
    let base=view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0)+local;
    let stride=if interleaved {view.get("byteStride").and_then(Value::as_u64).unwrap_or(element)} else {element};
    for c in 0..columns {
        for row in 0..rows {
            if output.len()>=262144 { return Err(ApiError::bad("GLB scalar checker budget exceeded")); }
            output.push(Scalar {offset:base+c*column+row*width,stride,remaining:count,width:width as usize,
                rule,partial:[0;4],used:0,previous:None});
        }
    }
    Ok(())
}
impl<R:Read> Read for Checked<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count=self.reader.read(bytes)?;
        let end=self.position+count as u64;
        while self.pending.last().is_some_and(|scalar|scalar.offset<end) {
            self.active.push(self.pending.pop().unwrap());
        }
        for scalar in &mut self.active {scalar.observe(self.position,&bytes[..count],&mut self.times)?;}
        self.active.retain(|scalar|scalar.remaining>0);
        self.position=end;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;
    struct OneByte<R>(R);
    impl<R:Read> Read for OneByte<R> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let count=bytes.len().min(1);
            self.0.read(&mut bytes[..count])
        }
    }
    fn check(doc: &Value, bytes: Vec<u8>) -> bool {
        let mut reader=Checked::new(OneByte(Cursor::new(bytes)),doc).unwrap();
        std::io::copy(&mut reader,&mut std::io::sink()).is_ok() && reader.finish().is_ok()
    }
    #[test]
    fn streaming_checks_indices_floats_and_split_scalar_reads() {
        let doc=json!({"bufferViews":[{"byteLength":36},{"byteOffset":36,"byteLength":6}],
            "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":3},
                {"bufferView":1,"componentType":5123,"type":"SCALAR","count":3}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}]});
        let mut bytes=vec![0;36];
        bytes.extend([0u16,1,2].into_iter().flat_map(u16::to_le_bytes));
        assert!(check(&doc,bytes.clone()));
        bytes[40..42].copy_from_slice(&3u16.to_le_bytes());
        assert!(!check(&doc,bytes.clone()));
        bytes[40..42].copy_from_slice(&2u16.to_le_bytes());
        bytes[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(!check(&doc,bytes));
    }
    #[test]
    fn sparse_indices_must_be_sorted_unique_and_in_range() {
        let doc=json!({"bufferViews":[{"byteLength":4},{"byteOffset":4,"byteLength":24}],
            "accessors":[{"componentType":5126,"type":"VEC3","count":3,"sparse":{"count":2,
                "indices":{"bufferView":0,"componentType":5123},"values":{"bufferView":1}}}]});
        let mut bytes=vec![0;28];
        bytes[2..4].copy_from_slice(&2u16.to_le_bytes());
        assert!(check(&doc,bytes.clone()));
        bytes[2..4].copy_from_slice(&0u16.to_le_bytes());
        assert!(!check(&doc,bytes.clone()));
        bytes[2..4].copy_from_slice(&3u16.to_le_bytes());
        assert!(!check(&doc,bytes));
    }
    #[test]
    fn sparse_animation_times_are_checked_after_overlay() {
        let doc=json!({"bufferViews":[{"byteLength":3},{"byteOffset":4,"byteLength":12},{"byteOffset":16,"byteLength":36}],
            "accessors":[{"componentType":5126,"type":"SCALAR","count":3,"sparse":{"count":3,
                "indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}},
                {"bufferView":2,"componentType":5126,"type":"VEC3","count":3}],
            "animations":[{"samplers":[{"input":0,"output":1}]}]});
        let mut bytes=vec![0;52];
        bytes[..3].copy_from_slice(&[0,1,2]);
        bytes[8..12].copy_from_slice(&1f32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2f32.to_le_bytes());
        assert!(check(&doc,bytes.clone()));
        bytes[12..16].copy_from_slice(&1f32.to_le_bytes());
        assert!(!check(&doc,bytes));
    }
}
