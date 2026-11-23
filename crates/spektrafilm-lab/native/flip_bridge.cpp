#include <FLIP.h>
#include <cstddef>
#include <cmath>
#include <climits>
#include <vector>
extern "C" int spektrafilm_flip(const float* reference,const float* test,std::size_t count,std::size_t width,std::size_t height,float ppd,float* output){
 if(!reference||!test||!output||count!=width*height||!width||!height||width>INT_MAX||height>INT_MAX||!std::isfinite(ppd)||ppd<=0) return -1;
 try {
 std::vector<float> a(reference,reference+count*3),b(test,test+count*3);
 for(std::size_t i=0;i<count*3;i++){a[i]=FLIP::color3::sRGBToLinearRGB(a[i]);b[i]=FLIP::color3::sRGBToLinearRGB(b[i]);}
 FLIP::Parameters p;p.PPD=ppd;float mean=0;float* map=nullptr;
 FLIP::evaluate(a.data(),b.data(),(int)width,(int)height,false,p,false,true,mean,&map);
        if (!map) return -2;
        for (std::size_t i = 0; i < count; i++) output[i] = map[i];
        delete[] map;
        return 0;
 }catch(...){return -3;}
}
