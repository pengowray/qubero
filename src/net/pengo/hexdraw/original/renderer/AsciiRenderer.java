package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

public class AsciiRenderer extends SeperatorRenderer {

    public AsciiRenderer(FontMetrics fm, int columnCount, boolean render) {
        super(fm, columnCount, render);
    }
    
    public static String byte2ascii(byte b)  {
        //if (b <= 0x20)
        //return " ";
        
        //return Character.toString((char)b); //FIXME: do this some other way?
        return (char)(((int)b) & 0xff) + "";
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
        return "Ascii";
    }

}
