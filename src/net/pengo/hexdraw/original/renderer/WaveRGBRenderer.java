package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.HexPanel;

public class WaveRGBRenderer extends SeperatorRenderer {

    public WaveRGBRenderer(HexPanel hexpanel, boolean render) {
        super(hexpanel, render);
        setEnabled(render);
    }

    public int renderBytes( Graphics g, int hexStart[], long lineNumber, byte ba[], boolean selecta[], int length ) {
        //// draw side wave form:
        FontMetrics fm = g.getFontMetrics();
        
        int charsHeight = fm.getHeight();        
        
        int waveWidth = 128;
        final int colourChannels = 3;
        int count = length / colourChannels;
        int offset;
		int col[] = new int[colourChannels<3 ? 3 : colourChannels ];
 
        for ( int i=0; i < count; i++ ) {

			offset = i * colourChannels;	        
            float xPt = (float)waveWidth / 255.0f / colourChannels; // y point distance
	        float yPt = colourChannels * (float)charsHeight / (float)length ; // y point distance
	        //int waveOffset = (int) (charsHeight + (yPt*i));
	        
	        for (int j = 0; j < colourChannels; j++) {
	            col[(int)((lineNumber+offset+j) % colourChannels)] = (int)ba[offset+j] & 0xff;
	        }

//	        int colr = ((int)ba[offset] & 0xff);
//	        int colg = ((int)ba[offset+1] & 0xff);
//	        int colb = ((int)ba[offset+2] & 0xff);
	        //int cola = ((int)ba[offset+3] & 0xff);
	        
	        if (selecta[i]) {     
	            g.setColor(new Color(170,170,255));
	        } else {
	            g.setColor(new Color(col[0], col[1], col[2]));
	        }
	        //g.fillRect(0, (int)(i *yPt), (int)((col[0] + col[1] + col[2] ) * xPt) , (int)(yPt+.5));
			g.fillRect(0, (int)(i *yPt), waveWidth , (int)(yPt+.5));
        }
        //g.drawLine(0, waveTopHeight+waveOffset, 0+coll, waveTopHeight+waveOffset);
        return waveWidth;
    }
    
    public String toString() {
        return "Wave RGB";
    }

}
