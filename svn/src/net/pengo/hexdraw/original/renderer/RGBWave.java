/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/*
 * Created on Jan 11, 2004
 */
package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

/**
 * @author Peter Halasz
 *
 * This class is a bit of a hack.. wont work with very diverse settings..
 * 
 * Should show alternating R,G and B "wave" lines (unsigned)
 * (red, green and blue line lengths according to rgb values)
 *  
 */

public class RGBWave extends SeperatorRenderer {
    int colourChannels = 3; // max 4.. otherwise extend col[]
    int sampleBits = 8;
    private int waveWidth = 256;
    Color col[] = new Color[]{Color.red, Color.green, Color.blue, Color.black};
    
    /**
     * @param fm
     * @param columnCount
     * @param render
     */
    public RGBWave(FontMetrics fm, int columnCount, boolean render, int colourChannels) {
        super(fm, columnCount, render);
        this.colourChannels = colourChannels;
    }

    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor) {
        //// draw side wave form:
        
        int charsHeight = fm.getHeight();
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
            b &= 0xFF; 
                
            //System.out.println("i:"+i +" b:"+b);
            
            //coll += (2 << (sampleBytes * 8))/2;
            
            if (selecta[i])  {     
                g.setColor(new Color(170,170,255));
            } else {
                //g.setColor(Color.darkGray);
                g.setColor(col[(int)(lineNumber + i) % colourChannels]);
                
            }
            //g.fillRect((waveWidth/2), (int)(i * sampleBytes * yPt), (int)(coll*xPt)-(waveWidth/2), (int)yPt * sampleBytes);
            //g.fillRect((int)(128 *xPt), (int)(i * sampleBits * yPt), (int)(b*xPt), (int)yPt * sampleBits);
            //g.setColor(Color.red);
            g.fillRect(0, (int)(i * yPt), b, (int)yPt);
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
        return "RGB Wave " + colourChannels +" (unsigned)";
    }    

}
