package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.Graphics;

import net.pengo.hexdraw.original.HexPanel;

public class WaveRenderer extends SeperatorRenderer {
    int sampleBits;
    
    public WaveRenderer(HexPanel hexpanel, boolean render) {
        this(hexpanel,8,render);
    }
    
    public WaveRenderer(HexPanel hexpanel, int sampleBits, boolean render) {
        super(hexpanel, render);
        setEnabled(render);
        this.sampleBits = sampleBits;
    }

    public int renderBytes( Graphics g, long lineNumber,
        byte ba[], int baOffset, int baLength, boolean selecta[],
        int columnWidth ) {
        //// draw side wave form:
        int waveWidth = 256;
        int charsHeight = 16;
        int imax =  8 * baLength / (sampleBits);
        
        for ( int i=0; i < imax; i++ ) {
	        
            float xPt = 1; //(float)waveWidth; // / (2 << (sampleBits));
	        float yPt = charsHeight/(float)imax;// / (2 << sampleBits); // y point distance
	        //int waveOffset = (int) (charsHeight + (yPt*i*sampleBits));
	        
	        //int coll = (int)( ((( 128 + (int)ba[i] & 0xff)) % 256) *xPt);
	        //int coll = (int)( ((  (int)ba[i] & 0xff)) *xPt);
	         
	        int b;
	        
	        //b = ba[(baOffset + (i*(sampleBits/8)))];
	        //b = ba[(baOffset + (i*(sampleBits/8)))];
	        b = ba[(baOffset + (sampleBits/8) -1 + (i*(sampleBits/8)))];
	        
	        System.out.println("i:"+i +" b:"+b);
	        
	        //coll += (2 << (sampleBytes * 8))/2;
	        
	        if (selecta[i])  {     
	            g.setColor(new Color(170,170,255));
	        } else {
	            g.setColor(Color.darkGray);
	        }
	        //g.fillRect((waveWidth/2), (int)(i * sampleBytes * yPt), (int)(coll*xPt)-(waveWidth/2), (int)yPt * sampleBytes);
	        //g.fillRect((int)(128 *xPt), (int)(i * sampleBits * yPt), (int)(b*xPt), (int)yPt * sampleBits);
	        if (b>0) {
	            //g.setColor(Color.red);
	            g.fillRect((int)128, (int)(i * yPt), (int)b, (int)yPt);
	        } else {
	            //g.setColor(Color.blue);
	            g.fillRect((int)128+b , (int)(i * yPt), (int)-b, (int)yPt);
	        }
        }
        //g.drawLine(0, waveTopHeight+waveOffset, 0+coll, waveTopHeight+waveOffset);
        return waveWidth;
    }
    
    public String toString() {
        return "Wave" + sampleBits;
    }

}
