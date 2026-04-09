mat3 rotx(float a) { mat3 rot; rot[0] = vec3(1.0, 0.0, 0.0); rot[1] = vec3(0.0, cos(a), -sin(a)); rot[2] = vec3(0.0, sin(a), cos(a)); return rot; }
mat3 roty(float a) { mat3 rot; rot[0] = vec3(cos(a), 0.0, sin(a)); rot[1] = vec3(0.0, 1.0, 0.0); rot[2] = vec3(-sin(a), 0.0, cos(a)); return rot; }
mat3 rotz(float a) { mat3 rot; rot[0] = vec3(cos(a), -sin(a), 0.0); rot[1] = vec3(sin(a), cos(a), 0.0); rot[2] = vec3(0.0, 0.0, 1.0); return rot; }

const float mouseRotateSpeed = 5.0;

struct sdObject
{
    vec3 pos;
    float rad;
    int index;
};

// Amount of butterflies    
#define OBJECTS 40
    
sdObject sdObjects[OBJECTS];    

// objects that will be potentially hit, found at prestep
#define CACHED 5
sdObject cachedObjects[CACHED];

int maxCacheIndex = 0;
    
// distance functions from https://iquilezles.org/articles/distfunctions

float udBox( vec3 p, vec3 b )
{
  return length(max(abs(p)-b,0.0));
}

float sdHexPrism( vec3 p, vec2 h )
{
    vec3 q = abs(p);
    return max(q.z-h.y,max((q.x*0.866025+q.y*0.5),q.y)-h.x);
}
//// 

const float MATERIAL_BODY = 0.0;
const float MATERIAL_WING = 1.0;


const float OBJECT_SIZE = 0.5;    
vec2 getModel(in vec3 pos, int index)
{
    float phase = float(index);
    float l = length(pos);
	
    float bl = (sin(pos.z * 12.0 - 5.0) * 0.5 + 0.5) + 0.3;
    float body = sdHexPrism(pos - vec3(0.0, 0.0, 0.), vec2(OBJECT_SIZE * 0.04 * bl, OBJECT_SIZE * 0.2));         

    float wx = max(abs(l * 6. + .2) - .4, 0.0);
    float sl = 1.5* abs(sin(wx)) + 0.05;
    
    vec3 wing = vec3(OBJECT_SIZE * 0.5, OBJECT_SIZE * 0.01, OBJECT_SIZE * 0.25 * sl);
    
    float w1 = udBox(rotz(sin(iTime * 22.0 + phase)) * pos - vec3(OBJECT_SIZE * 0.5, OBJECT_SIZE * 0., 0.0), wing);
    float w2 = udBox(rotz(-sin(iTime * 22.0 + phase)) * pos + vec3(OBJECT_SIZE * 0.5, OBJECT_SIZE * 0., 0.0), wing);
    
    float id = MATERIAL_BODY;
    if(w1 < body || w2 < body)
    {
        id = MATERIAL_WING;
    }
    
    float m = min(body, w1);
    m = min(m, w2);
    
    return vec2(m, id);
            
}


vec2 map(in vec3 rp, in sdObject[CACHED] objects, inout vec3 localPos, inout int index)
{
    float m = 9999.0;
    vec2 ret = vec2(m, 0.0);
    
    for (int i = 0; i < CACHED; ++i)
    {
        if ( i <= maxCacheIndex)
        {
            vec3 lp = rp - objects[i].pos;
            vec2 mat = getModel(lp, objects[i].index);     
            float a = min(mat.x, m);
            if (a < m)
            {
                m = a;
                ret = mat;
                localPos = lp;
                index = objects[i].index;
            }
        }
    }
    return ret;
}

/**
 * In the pre-step function I find objects that will be potentially hit.
 * I will check only against these objects at later steps.
 */
float prestep(in vec3 ro, in vec3 rp, in vec3 rd, in vec3 rd90degXAxis, in vec3 rd90degYAxis)
{
    maxCacheIndex = -1;
    float m = 99999.0;
    for (int i = 0; i < OBJECTS; ++i)
    {
        vec3 sp = -ro + sdObjects[i].pos;
        
        float distToPlaneY = abs(dot(rd90degYAxis, sp));
    	float distToPlaneX = abs(dot(rd90degXAxis, sp));
        
        float distanceToPlanes = max(distToPlaneY, distToPlaneX);
        distanceToPlanes = distanceToPlanes - sdObjects[i].rad;

        vec2 mat = getModel(rp - sdObjects[i].pos * (1.0 + distanceToPlanes), sdObjects[i].index);
        float l = mat.x;
        m = min(m, l);
        
        if(distanceToPlanes <= 0.0 && ++maxCacheIndex < CACHED)
        {
            if (maxCacheIndex == 0) cachedObjects[0] = sdObjects[i];
            else if (maxCacheIndex == 1) cachedObjects[1] = sdObjects[i];
            else if (maxCacheIndex == 2) cachedObjects[2] = sdObjects[i];
            else if (maxCacheIndex == 3) cachedObjects[3] = sdObjects[i];
            else if (maxCacheIndex == 4) cachedObjects[4] = sdObjects[i];
            else return m;    
            
        }
    }
    
    return m;
}



void trace(in vec3 rp, in vec3 rd, inout vec4 color)
{
    vec3 ro = rp;
    float travel = 0.0;
    const int STEPS = 50;
    // build orthonormal frame to get right and up vectors to be used in distance calculations 
    vec3 tmp = normalize(cross(rd, vec3(0.0, 1.0, 0.0)));
    vec3 up = normalize(cross(rd, tmp));
    vec3 right = cross(rd, up);
    
    // pre-step and move ray
    travel = prestep(ro, rp, rd, right, up);
    rp += travel * rd;
    
    vec3 local = vec3(0.0);
    int hitindex = 0;
    
    for (int i = 0; i < STEPS; ++i)
    {
       vec2 mat = map(rp, cachedObjects, local, hitindex);
       float dist = mat.x;
        
       if(dist <= 0.0)
       {
           float id = mat.y;
           float indx = float(hitindex);
           float c1 = sin(indx * 0.1) * 0.5 + 0.5;
           float c2 = abs(cos(abs(local.z * 15.0)) + sin(abs(local.x) * 15.0));
           float c3 = cos(indx * 0.4);
           color = vec4(mat.y, c2 * mat.y, c1 * mat.y, 1.0) * abs(sin(indx * 0.1));
           color.a = 1.0;
               
           return;
       }
       float dst = max(0.01, dist);
       travel += dst;
       rp += rd * dst;
       if(travel > 30.0) return;
    }
}



void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    fragColor = vec4(0.0);
	vec2 uv = fragCoord.xy / iResolution.xy;
    uv -= vec2(0.5);
    uv.y /= iResolution.x / iResolution.y;
    vec2 mouse = iMouse.xy / iResolution.xy;
    mouse -= vec2(0.5);
    
    if(mouse.xy == vec2(-0.5))
    {
        mouse = vec2(0.0);
    }
    
    mouse *= mouseRotateSpeed;
    
    for (int i = 0; i < OBJECTS; ++i)
    {
        vec3 p = (texture(iChannel0, sin(iTime * 0.001) + 0.21 * vec2(float(i))) - vec4(0.5)).rgb;
        p *= roty(iTime * 2.0);
        p.z += (sin(iTime) * 0.5 + 0.5) * 1.0;
        p.x *= 1.0 + (sin(iTime * 0.1) * 0.5 + 0.5) * 0.25;
        p.y *= 1.0 + (cos(iTime * 0.1) * 0.5 + 0.5) * 0.25;
        sdObjects[i] = sdObject(p * 10.0, OBJECT_SIZE * 1. , i);
    }
    
    vec3 rp = vec3(0.0, .0, 1.0);
    vec3 rd = normalize(vec3(uv, 0.3));
    
    rd *= rotx(mouse.y);
    rd *= roty(mouse.x);
    
    trace(rp, rd, fragColor);
    fragColor = mix(fragColor, texture(iChannel1, rd * roty(3.14159 * 0.5)), 1.0 - fragColor.a);
    
    float luma = (fragColor.r + fragColor.g + fragColor.b) * 0.33;
    fragColor -= (luma) * vec4(.9, .5, .0, 1.) * clamp(rd.y - 0.05, 0.0, 1.0);
    
    fragColor += vec4(.2, 0.4, 0.0, 0.0) * abs(clamp(rd.y, -1.0, .0));
	
    // frame
    fragColor = mix(fragColor, vec4(0.0), 1.0 - smoothstep(0.5, 0.45, abs(uv.x)));
    fragColor = mix(fragColor, vec4(0.0), 1.0 - smoothstep(0.28, 0.2, abs(uv.y)));
    
}