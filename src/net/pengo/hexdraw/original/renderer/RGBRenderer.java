package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

public class RGBRenderer extends SeperatorRenderer {
    int colourChannels = 4;
    
    public RGBRenderer(FontMetrics fm, int columnCount, int colourChannels, boolean render) {
        super(fm,columnCount,render);

        this.colourChannels = colourChannels;
    }
    
    public RGBRenderer(FontMetrics fm, int columnCount, boolean render) {
        this(fm,columnCount,3,render);
    }

    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor) {
    	//// draw side wave form:
        
        int charsHeight = fm.getHeight();        
        int barWidth = charsHeight;
        
        int count = baLength / colourChannels;
        int offset = baOffset;
        
        // array size minimum of 3
		int col[] = new int[colourChannels<3 ? 3 : colourChannels ];
 
		
        for ( int i=0; i < count; i++ ) { //

			offset = i * colourChannels;	        
            float xPt = (float)barWidth / 255.0f / colourChannels; // y point distance
	        float yPt = colourChannels * (float)charsHeight / (float)columnCount; // y point distance
	        //int waveOffset = (int) (charsHeight + (yPt*i));
	        
	        for (int j = 0; j < colourChannels; j++) {
	            col[(int)((lineNumber+offset+j) % colourChannels)] = (int)ba[offset+j] & 0xff;
	        }

	        if (selecta[i]) {     
	            g.setColor(new Color(170,170,255));
	        } else {
	            g.setColor(new Color(col[0], col[1], col[2]));
	        }
	        //g.fillRect(0, (int)(i *yPt), (int)((col[0] + col[1] + col[2] ) * xPt) , (int)(yPt+.5));
			g.fillRect(0, (int)(i *yPt), barWidth, (int)(yPt+.5));
        }
        //g.drawLine(0, waveTopHeight+waveOffset, 0+coll, waveTopHeight+waveOffset);
        //return barWidth;
    }
    
    public int getWidth() {
        return fm.getHeight();
    }
    
    public long whereAmI(int x, int y, long lineAddress){
        int charsHeight = fm.getHeight(); 
        float yPt = colourChannels * (float)charsHeight / (float)columnCount;
        
        return (int)(y / yPt) + lineAddress;
    }    
    public String toString() {
        return "RGB " + colourChannels + " (unsigned)";
    }

}
