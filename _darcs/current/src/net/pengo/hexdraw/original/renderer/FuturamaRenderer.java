package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

public class FuturamaRenderer extends SeperatorRenderer {

    private boolean hideHighAscii; // hide ascii > 127 and control characters (0
    
    public FuturamaRenderer(FontMetrics fm, int columnCount, boolean render) {
	super(fm, columnCount, render);
    }
    
    private String byte2ascii(byte b)  {
        //return Character.toString((char)b); //FIXME: do this some other way?

	int unsigned = (((int)b) & 0xff);
	//Character.is

        if (hideHighAscii) {
	    if (unsigned < 0x20 || unsigned >= 0x7F)
		return "·";
	}
	
        return (char)unsigned + "";
    }
    
    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor){

        //// draw ascii:
        FontMetrics fm = g.getFontMetrics();
        
        int charsWidth = fm.charWidth('w'); 
        int charsHeight = fm.getHeight();
        
        
        for ( int i=0; i < baLength; i++ ) {
            if (selecta[i])  {
                //g.setColor( GUI.selectionColor() );
                g.setColor( new Color(170,170,255) ); // FIXME: cache
                g.fillRect( i * charsWidth, 0, charsWidth, charsHeight);  //FIXME: precision loss!
            }
            g.setColor(Color.black);
            //g.setColor(Color.darkGray);
            g.drawString( byte2ascii(ba[baOffset+i]), i * charsWidth, fm.getAscent()); //FIXME: precision loss!
        }
        
        //return columnCount * charsWidth;
    }
    
    public int getWidth() {
        return fm.charWidth('W')*columnCount;
    }

    public long whereAmI(int x, int y, long lineAddress){
        return lineAddress + (x / fm.charWidth('W'));
    }
    
    public String toString() {
	if (!hideHighAscii)
	    return "Ascii (extended)";
	
	return "Ascii";
    }

}
