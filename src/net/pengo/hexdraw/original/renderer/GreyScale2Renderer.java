package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.HexPanel;

public class GreyScale2Renderer extends SeperatorRenderer {

    public GreyScale2Renderer(HexPanel hexpanel, boolean render) {
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
        
        int col, col2;
        
        for ( int i=0; i < baLength; i++ ) {
            col = ~((int)ba[i]) & 0xf0; // the gamma of left/outer square
            col2 = (~((int)ba[baOffset+i]) & 0x0f) << 4; // the gamma of right/inner square
            col = col | (col >> 4); // turn a0 into aa
            col2 = col2 | (col2 >> 4);
            
            if (selecta[i])
                g.setColor(new Color(col/3*2,col/3*2,col/2+128));
            else
                g.setColor(new Color(col,col,col));
            
            g.fillRect(i *charsWidth, 0, charsWidth-1, charsHeight-1);
            
            if (selecta[i])
                g.setColor(new Color(col2/3*2,col2/3*2,col2/2+128));
            else
                g.setColor(new Color(col2,col2,col2));
            
            g.fillRect((i * charsWidth) + (charsWidth/2), (int)(charsHeight/2), (charsWidth/2)-1, (charsHeight/2)-1);
            //g.fillRect(i *charsWidth, 0, charsWidth, charsHeight);
        }
        
        return columnWidth * charsWidth;
    }

    public String toString() {
        return "Grey scale II";
    }

}
