package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.HexPanel;

public class GreyScaleRenderer extends SeperatorRenderer {

    public GreyScaleRenderer(HexPanel hexpanel, boolean render) {
        super(hexpanel, render);
        setEnabled(render);
    }

    public int renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            int columnWidth ) {
    	//// draw grey scale:
        FontMetrics fm = g.getFontMetrics();
        
        int charsHeight = fm.getHeight();
        int charsWidth = charsHeight;
        
        int col;
        
        for ( int i=0; i < baLength; i++ ) {
	        col = ~((int)ba[baOffset+i]) & 0xff; // the "gamma" of grey squares
	        if (selecta[i])  {
	            g.setColor(new Color(col/3*2,col/3*2,col/2+128));
	        }
	        else  {
	            g.setColor(new Color(col,col,col));
	        }
	        g.fillRect(i *charsWidth, 0, charsWidth, charsHeight);
        }
        
        return columnWidth * charsWidth;
    }
    
    public String toString() {
        return "Grey scale";
    }

}
