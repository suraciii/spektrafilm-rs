struct Params {total:u32,depth:u32,first:u32,count:u32,}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> children:array<f32>;
@group(0) @binding(2) var<storage,read_write> parents:array<f32>;
fn node_count(index:u32)->u32{
  var count=p.total;
  for(var level=p.depth;level>0u;level--){
    let right=((index>>(level-1u))&1u)!=0u;
    if(count<=128u){if(right){return 0u;}}
    else{let middle=(count/2u)&~7u;if(right){count-=middle;}else{count=middle;}}
  }
  return count;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
  let id=gid.x+gid.y*grid.x*256u;
  if(id>=p.count*6u){return;}
  let parent=id/6u;let component=id%6u;
  let count=node_count(p.first+parent);
  let left=children[parent*12u+component];
  if(count==0u){parents[id]=-0.0;}
  else if(count<=128u){parents[id]=left;}
  else{parents[id]=left+children[parent*12u+6u+component];}
}
