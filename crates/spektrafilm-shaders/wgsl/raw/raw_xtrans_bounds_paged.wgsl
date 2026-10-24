// The native green-bounds scan, paused only at a window edge. Cursor values
// are control state; all min/max work and every sensor read remain on device.
struct RawWindow { x:u32, y:u32, width:u32, height:u32 };
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> window:RawWindow;
@group(0) @binding(2) var<storage,read_write> image:array<u32>;
@group(0) @binding(3) var<storage,read> cfa:array<u32>;
@group(0) @binding(4) var<storage,read_write> metadata:array<i32>;
// x,y,low,high,retries,done,padding,padding.
@group(0) @binding(5) var<storage,read_write> cursor:array<u32>;
fn color(y:i32,x:i32)->u32{return cfa[u32((y+6)%6)*6u+u32((x+6)%6)];}
fn hx(y:i32,x:i32,k:i32)->i32{return metadata[u32(((y%3)*3+x%3)*16+k)];}
fn at(p:i32)->u32{return u32(p)-window.x;}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>){
    if(gid.x!=0u || metadata[146]!=0){return;}
    var x=i32(cursor[0]);var y=i32(cursor[1]);var low=cursor[2];var high=cursor[3];var retries=cursor[4];
    loop {
        if(x>=i32(frame.width)-2){x=2;y++;low=65535u;high=0u;}
        if(y>=i32(frame.height)-2){cursor[5]=1u;break;}
        // allhex image offsets are native signed shorts, including stride
        // truncation on wide images. Keep their actual flat read interval.
        let first=max(0,y*i32(frame.width)+x-32768);
        let last=min(i32(raw_pixel_count(frame))-1,y*i32(frame.width)+x+32767);
        if(first<i32(window.x) || last>=i32(window.x+window.width)){break;}
        let p=y*i32(frame.width)+x;
        if(color(y,x)==1u){low=65535u;high=0u;x++;continue;}
        if(high==0u){for(var c=0;c<6;c++){let value=image[at(p+hx(y,x,c))*4u+1u];low=min(low,value);high=max(high,value);}}
        image[at(p)*4u+1u]=low;image[at(p)*4u+3u]=high;
        switch((y-metadata[144])%3){
            case 1:{if(y<i32(frame.height)-3){y++;x--;}}
            case 2:{low=65535u;high=0u;x+=2;if(x<i32(frame.width)-3 && y>2){y--;retries++;if(retries>raw_pixel_count(frame)){metadata[146]=1;break;}}}
            default:{}
        }
        if(metadata[146]!=0){break;}
        x++;
    }
    cursor[0]=u32(x);cursor[1]=u32(y);cursor[2]=low;cursor[3]=high;cursor[4]=retries;
}
