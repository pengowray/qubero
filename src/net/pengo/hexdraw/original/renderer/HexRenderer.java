package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.HexPanel;

public class HexRenderer extends SeperatorRenderer {
// add groupings width
    public HexRenderer(HexPanel hexpanel, boolean render) {
        super(hexpanel, render);
        setEnabled(render);
    }
    
    private static String byte2hex(byte b)  {
        int l = ((int)b & 0xf0) >> 4;
        int r = (int)b & 0x0f;
        
        return ""
        + (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
        + (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
    }
    
    
    public int renderBytes( Graphics g, long lineNumber,
            				byte ba[], int baOffset, int baLength, boolean selecta[],
            				int columnWidth ) {
        //// draw hex:
        
        FontMetrics fm = g.getFontMetrics();
        int charsHeight = fm.getHeight();
        int charsWidth = fm.charWidth('w') *3;
        
        //( length >0 ? hexStart[1]-hexStart[0] : 0 );
        //g.drawString(addr, 0,  );
        
        for ( int i=0; i < baLength; i++ ) {
            if (selecta[i])  {
                //g.setColor( GUI.selectionColor() );
                g.setColor( new Color(170,170,255) );
                g.fillRect( i * charsWidth, 0, charsWidth, charsHeight);
            }
            g.setColor(Color.black);
            //g.setColor(Color.darkGray);
            g.drawString( byte2hex(ba[baOffset+i]), i * charsWidth, fm.getAscent());
        }
        
        return columnWidth * charsWidth;
        //// draw address:
        //FontMetrics fm = g.getFontMetrics();
        //String addr = BigInteger.valueOf(lineNumber * length).toString(16)+":";
        //g.drawString(addr, 0, fm.getAscent() );
        //return fm.bytesWidth(addr.getBytes(), 0, addr.length()); 
    }
    
    public String toString() {
        return "Hex";
    }

}
