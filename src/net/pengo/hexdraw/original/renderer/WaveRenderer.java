package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.Graphics;

import net.pengo.hexdraw.original.HexPanel;

public class WaveRenderer extends SeperatorRenderer {

    public WaveRenderer(HexPanel hexpanel, boolean render) {
        super(hexpanel, render);
        setEnabled(render);
    }

    public int renderBytes( Graphics g, int hexStart[], long lineNumber, byte ba[], boolean selecta[], int length ) {
        //// draw side wave form:
        int waveWidth = 128;
        int charsHeight = 16;
        
        
        for ( int i=0; i < length; i++ ) {
	        
            float xPt = (float)waveWidth / 255.0f; // y point distance
	        float yPt = 16.0f / (float)charsHeight; // y point distance
	        int waveOffset = (int) (charsHeight + (yPt*i));
	        
	        int coll = (int)( ((int)ba[i] & 0xff) *xPt);
	        
	        if (selecta[i])  {     
	            g.setColor(new Color(170,170,255));
	        } else {
	            g.setColor(Color.darkGray);
	        }
	        g.fillRect(0, (int)(i *yPt), coll, (int)yPt);
        }
        //g.drawLine(0, waveTopHeight+waveOffset, 0+coll, waveTopHeight+waveOffset);
        return waveWidth;
    }
    
    public String toString() {
        return "Wave";
    }

}
