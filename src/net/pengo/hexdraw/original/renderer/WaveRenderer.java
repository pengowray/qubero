package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

public class WaveRenderer extends SeperatorRenderer {
    private int sampleBits;
    private int waveWidth = 256;
    
    public WaveRenderer(FontMetrics fm, int hexPerLine, boolean render) {
        this(fm,hexPerLine,8,render);
    }
    
    public WaveRenderer(FontMetrics fm, int columnCount, int sampleBits, boolean render) {
        super(fm,columnCount,render);

        this.sampleBits = sampleBits;
    }

    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor) {
        //// draw side wave form:
        
        int charsHeight = fm.getHeight(); // 16
        int imax =  8 * baLength / (sampleBits);
        
        for ( int i=0; i < imax; i++ ) {
	        
            float xPt = 256 / (float)waveWidth; 
	        float yPt = charsHeight/(float)imax;// / (2 << sampleBits); // y point distance
	        //int waveOffset = (int) (charsHeight + (yPt*i*sampleBits));
	        
	        //int coll = (int)( ((( 128 + (int)ba[i] & 0xff)) % 256) *xPt);
	        //int coll = (int)( ((  (int)ba[i] & 0xff)) *xPt);
	         
	        int b;
	        
	        //b = ba[(baOffset + (i*(sampleBits/8)))];
	        //b = ba[(baOffset + (i*(sampleBits/8)))];
	        b = ba[(baOffset + (sampleBits/8) -1 + (i*(sampleBits/8)))];
	        
	        //System.out.println("i:"+i +" b:"+b);
	        
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
	            g.fillRect((int)(128*xPt), (int)(i * yPt), (int)(b*xPt), (int)yPt);
	        } else {
	            //g.setColor(Color.blue);
	            g.fillRect((int)((128+b)*xPt) , (int)(i * yPt), (int)(-b*xPt), (int)yPt);
	        }
        }
        //g.drawLine(0, waveTopHeight+waveOffset, 0+coll, waveTopHeight+waveOffset);
        
        //return waveWidth;
    }

    public int getWidth() {
        return waveWidth;
    }
    
    public long whereAmI(int x, int y, long lineAddress){
        int charsHeight = fm.getHeight(); 
        float yPt = (sampleBits/8) * (float)charsHeight / (float)columnCount;
        
        return (int)(y / yPt) + lineAddress;
    }    
    
    public String toString() {
        return "Wave " + sampleBits +" (signed)";
    }

}
