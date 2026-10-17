#include <math.h>
#include <stdio.h>
#include <stdlib.h>
struct float3 { float x, y, z; };
struct float2 { float x, y; };
static inline float3 make_float3(float x, float y, float z) { float3 r; r.x=x; r.y=y; r.z=z; return r; }
static inline float2 make_float2(float x, float y) { float2 r; r.x=x; r.y=y; return r; }
#define __DEVICE__ static inline
#define __CONSTANT__ static const
#define __CONSTANTREF__
#define _powf powf
#define _logf logf
#define _log2f log2f
#define _log10f log10f
#define _expf expf
#define _exp2f exp2f
#define _exp10f(x) powf(10.0f,(x))
#define _sqrtf sqrtf
#define _fmaxf fmaxf
#define _fminf fminf
#define _fabs fabsf
#define _fmod fmodf
#define _atan2f atan2f
#define _sinf sinf
#define _cosf cosf
#define _floor floorf
#define _ceil ceilf
#define _round roundf
#define _hypotf hypotf
#define _copysignf copysignf
#define _saturatef(x) fminf(fmaxf((x),0.0f),1.0f)
static inline float3 operator+(float3 a, float3 b){return make_float3(a.x+b.x,a.y+b.y,a.z+b.z);}
static inline float3 operator-(float3 a, float3 b){return make_float3(a.x-b.x,a.y-b.y,a.z-b.z);}
static inline float3 operator*(float3 a, float3 b){return make_float3(a.x*b.x,a.y*b.y,a.z*b.z);}
static inline float3 operator/(float3 a, float3 b){return make_float3(a.x/b.x,a.y/b.y,a.z/b.z);}
static inline float3 operator*(float3 a, float b){return make_float3(a.x*b,a.y*b,a.z*b);}
static inline float3 operator*(float b, float3 a){return make_float3(a.x*b,a.y*b,a.z*b);}
static inline float3 operator/(float3 a, float b){return make_float3(a.x/b,a.y/b,a.z/b);}
static inline float3 operator/(float b, float3 a){return make_float3(b/a.x,b/a.y,b/a.z);}
static inline float3 operator+(float3 a, float b){return make_float3(a.x+b,a.y+b,a.z+b);}
static inline float3 operator+(float b, float3 a){return make_float3(a.x+b,a.y+b,a.z+b);}
static inline float3 operator-(float3 a, float b){return make_float3(a.x-b,a.y-b,a.z-b);}
static inline float3 operator-(float b, float3 a){return make_float3(b-a.x,b-a.y,b-a.z);}
static inline float3 operator-(float3 a){return make_float3(-a.x,-a.y,-a.z);}
static inline float3& operator+=(float3& a, float3 b){a=a+b;return a;}
static inline float3& operator-=(float3& a, float3 b){a=a-b;return a;}
static inline float3& operator*=(float3& a, float3 b){a=a*b;return a;}
static inline float3& operator/=(float3& a, float3 b){a=a/b;return a;}
static inline float3& operator+=(float3& a, float b){a=a+b;return a;}
static inline float3& operator-=(float3& a, float b){a=a-b;return a;}
static inline float3& operator*=(float3& a, float b){a=a*b;return a;}
static inline float3& operator/=(float3& a, float b){a=a/b;return a;}
