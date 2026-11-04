// A parallel frontier of the unchanged global pairwise tree. Only a leaf
// crossing the end of this transport window publishes its accumulator state.
struct Params {
  total:u32, depth:u32, first_leaf:u32, leaves:u32,
  first_pixel:u32, pixels:u32, output_first:u32, pad1:u32,
}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> density:array<f32>;
@group(0) @binding(2) var<storage,read> sharpened:array<f32>;
@group(0) @binding(3) var<storage,read_write> carry:array<f32>;
@group(0) @binding(4) var<storage,read_write> sums:array<f32>;
fn node(index:u32)->vec2<u32>{
  var first=0u;var count=p.total;
  for(var level=p.depth;level>0u;level--){
    let right=((index>>(level-1u))&1u)!=0u;
    if(count<=128u){if(right){return vec2(first+count,0u);}}
    else{let middle=(count/2u)&~7u;if(right){first+=middle;count-=middle;}else{count=middle;}}
  }
  return vec2(first,count);
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
  let id=gid.x+gid.y*grid.x*256u;
  if(id>=p.leaves*3u){return;}
  let leaf=id/3u;let ch=id%3u;
  let range=node(p.first_leaf+leaf);let start=range.x;let total=range.y;
  let output=(p.first_leaf+leaf-p.output_first)*6u+ch;
  if(total==0u){sums[output]=-0.0;sums[output+3u]=-0.0;return;}
  let first=max(start,p.first_pixel);let last=min(start+total,p.first_pixel+p.pixels);
  let offset=first-start;let end=total-total%8u;
  var d:array<f32,8>;var s:array<f32,8>;
  if(offset!=0u){for(var k=0u;k<8u;k++){d[k]=carry[ch*16u+k];s[k]=carry[ch*16u+8u+k];}}
  var ds=-0.0;var ss=-0.0;
  if(total<8u){
    for(var j=first;j<last;j++){let i=(j-p.first_pixel)*3u+ch;ds=ds+max(density[i],0.0);ss=ss+sharpened[i];}
  }else{
    for(var j=first;j<min(last,start+end);j++){
      let coordinate=j-start;let k=coordinate%8u;let i=(j-p.first_pixel)*3u+ch;
      let dv=max(density[i],0.0);let sv=sharpened[i];
      if(coordinate<8u){d[k]=dv;s[k]=sv;}else{d[k]=d[k]+dv;s[k]=s[k]+sv;}
    }
    if(offset<end && last-start>=end){
      ds=((d[0]+d[1])+(d[2]+d[3]))+((d[4]+d[5])+(d[6]+d[7]));
      ss=((s[0]+s[1])+(s[2]+s[3]))+((s[4]+s[5])+(s[6]+s[7]));
    }else if(offset>=end){ds=d[0];ss=s[0];}
    for(var j=max(first,start+end);j<last;j++){let i=(j-p.first_pixel)*3u+ch;ds=ds+max(density[i],0.0);ss=ss+sharpened[i];}
    // Once the eight-lane prefix is complete, carry its scalar plus any tail.
    if(last-start>=end){d[0]=ds;s[0]=ss;}
  }
  sums[output]=ds;sums[output+3u]=ss;
  if(last<start+total){for(var k=0u;k<8u;k++){carry[ch*16u+k]=d[k];carry[ch*16u+8u+k]=s[k];}}
}
