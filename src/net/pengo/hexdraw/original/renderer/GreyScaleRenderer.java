package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

public class GreyScaleRenderer extends SeperatorRenderer {

    public GreyScaleRenderer(FontMetrics fm, int columnCount, boolean render) {
        super(fm, columnCount, render);
    }

    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor) {
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
        
        //return columnCount * charsWidth;
    }

    public int getWidth() {
        return fm.getHeight() * columnCount;
    }    
    
    public long whereAmI(int x, int y, long lineAddress){
        //height is same as width (square pieces)
        return lineAddress + (x / fm.getHeight());
    }     
    
    public String toString() {
        return "Grey scale (unsigned)";
    }

}
