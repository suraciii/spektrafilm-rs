// One invocation advances the original recurrence across page boundaries.
fn axis_add(a:f32,b:vec2<f32>)->f32 {
 let s=a+b.x;let v=s-a;let e=((a-(s-v))+(b.x-v))+0.0+b.y;
 let hi=s+e;return hi+(e-(hi-s));
}
struct Axis { range:vec4<u32>, step:vec4<u32> };
@group(0) @binding(0) var<uniform> p:Axis;
@group(0) @binding(1) var<storage,read_write> state:array<u32>;
@group(0) @binding(2) var<storage,read_write> destination:array<vec4<u32>>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>){
 if id.x!=0u{return;}
 var value=vec2<u32>(state[0],state[1]);
 for(var i=0u;i<p.range.y;i++){
  var sample=0u;
  if p.range.z==0u {
   sample=min(floor_double(value),p.range.w-1u);value=add_double(value,p.step.xy);
  }else{
   if p.range.x+i==p.range.w {value=vec2<u32>(p.step.z,0u);}
   sample=value.x;
   value.x=bitcast<u32>(axis_add(bitcast<f32>(value.x),bitcast<vec2<f32>>(p.step.xy)));
  }
  destination[i]=vec4<u32>(sample,0u,0u,0u);
 }
 state[0]=value.x;state[1]=value.y;
}
