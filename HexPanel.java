import java.awt.*;
import javax.swing.event.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.tree.*;

class HexPanel extends JPanel {
    protected DefNode defnode = null;
    protected RawData root;
    
    protected boolean dimensionsCalculated = false;
    protected int height; // height of entire "sheet" of hex
    protected int width; // width of entire "sheet" of hex
    protected int lineStart; // where does the first character start
    protected int hexPerLine = 16; // number of hex units shown per line
    protected int[] hexStart; // where each hex starts on a line.
    protected int unitWidth; // length of two characters, basically.
    protected int lineHeight; // height of one line of hex
    protected int totalLines;
    protected boolean charSizeKnown = false;
    protected int charWidth = 8, charHeight = 14; // probable sizes(?)
    
    protected RawDataSelection selection = null;
    protected int cursor = -1;

    private boolean draggingMode = false; // mouse is currently dragging a selection
    private boolean published = false; // has the current selection been published? 
    
    Font font = new Font("Monospaced", Font.PLAIN, 11); // antialias?
    
    public int hexFromClick(int x, int y) {

        int hx = -1;
        int hy = -1;

        for (int h=0; h<hexStart.length; h++) {
            if (x >= lineStart+hexStart[h] && x <= lineStart+hexStart[h]+unitWidth) {
                hx = h;
                break;
            }
        }
        if (hx == -1) {
            // not found
            return -1;
        }

        hy = y / lineHeight;

        return (hy * hexPerLine) + hx;      
    }

    public void setDefNode(DefNode defnode) {
	if (this.defnode != null) {
	    this.defnode.removeHexPanel(this); // old def node
	}

	this.defnode = defnode;
        this.root = defnode.getRawData();
	selection = null;
	cursor = -1;

        defnode.addHexPanel(this);

        dimensionsCalculated = false;
	calcDim();

	//XXX: need to repaint scrollbars too
	repaint();
    }
    
    public HexPanel(DefNode defnode) {
	super();
	setDefNode(defnode);
	
        //XXX: defnode should be used in rendering

	// MOUSE ROUTINES

        addMouseListener(
            new MouseInputAdapter() {
                public void mousePressed(MouseEvent e) {
                    // e.getModifiers(); // FIXME: later.
                    int hclick = hexFromClick( e.getX(), e.getY() );
                    if (hclick != -1) {
                        setSelection(hclick, 1, false);
			draggingMode = true;
                    } else {
			draggingMode = false;
                    }		    

                }

                public void mouseReleased(MouseEvent e) {
                    // e.getModifiers(); // FIXME: later.
		    if (draggingMode) {
			setSelection(getSelection(), true);
			draggingMode = false;
		    }

                }

                public void mouseClicked(MouseEvent e) {
                    // e.getModifiers(); // FIXME: later.
                    int hclick = hexFromClick( e.getX(), e.getY() );
                    if (hclick != -1) {
                        setSelection(hclick, 1, true);
                    }
		    draggingMode = false;
                }
            }
        );

        addMouseMotionListener(new MouseInputAdapter() {
            public void mouseDragged(MouseEvent e) {
                int hclick = hexFromClick( e.getX(), e.getY() );
                if (draggingMode && hclick != -1) {
		    int start = selection.getStart(); 
		    setSelection(start, (hclick-start)+1, false );
                }
        
            }
        });
        
    }
        
    public RawDataSelection getSelection() {
	return selection;
    }

    public void setSelection(RawDataSelection sel) {
	setSelection(sel, true);
    }

    public synchronized void setSelection(RawDataSelection sel, boolean publish) {
	if (sel.equals(selection) && (publish==false || published==true)) return;

	this.selection = sel;

	if (publish==true) {
	    defnode.selectionMade( new SelectionEvent(selection) );
	    published = true;
	} else {
	    published = false;
	}

        repaint(); //XXX: repaint only necessary areas!

    }

    public void setSelection(int offset, int len) {
	setSelection(offset, len, true);
    }
    public void setSelection(int offset, int len, boolean publish) {
        setSelection(new RawDataSelection(defnode, offset, len), publish);
    }

    public int getHeight() {
        return height;
   }

    public int getWidth() {
	return width;
    }
    
    public Dimension getPreferredSize() {
        //FIXME: this is a hack!

	return new Dimension(width, height);

    }

    public Dimension getMinimumSize() {
	return getPreferredSize();
    }

    public Dimension getSize() {
        return new Dimension(width, height);
    }


    public Dimension getMaximumSize() {
        return new Dimension(width, height);
    }
    
    //XXX: will need def listener?
 
    /** calculate dimensions */
    protected void calcDim() {
        if (dimensionsCalculated == true)
            return;

	calcDim(charWidth, charHeight); //XXX: may not have been worked out from FontMetrics
	if (charSizeKnown == false) {
	    // don't trust size until calculated with FontMetrics
	    dimensionsCalculated = false; 
	}
    }

    protected void calcDim(FontMetrics fm) {
        if (dimensionsCalculated == true)
            return;

	if (charSizeKnown == false) {
	    charWidth = fm.charWidth('W');
	    charHeight = fm.getHeight();
	    charSizeKnown = true;
	}

	calcDim(charWidth, charHeight);
    }

    protected synchronized void calcDim(int charWidth, int charHeight) {
        if (dimensionsCalculated == true)
            return;
        
        lineHeight = charHeight;
        lineStart = charWidth * 12; // ought to be enough space for hex address
        unitWidth = charWidth * 2; // length of two characters (eg FF).
        int asciiWidth = charWidth * hexPerLine; // how long the ascii stuff is

        int[] spaceEvery = new int[] {1, 2, 4, 8};
        int[] spaceSize = new int[]  {charWidth*3, 0, charWidth, charWidth };
        
        hexStart = new int[hexPerLine+1]; // where each hex starts on a line + where ascii starts
        
        int s = 0;
        for (int n=0; n<hexStart.length; n++) { // only place hexStart.length should be used. use hexPerLine instead.
            hexStart[n] = s;
            for (int m=0; m<spaceEvery.length; m++) {
                if ((n+1) % spaceEvery[m] == 0) {
                    s += spaceSize[m];
                }
            }
        }

        // if (root.isFixedLength()) {  // --- how to deal with non fixed length?
        
        totalLines = root.getLength() / hexPerLine;
        if ((float)totalLines != ((float)root.getLength() / hexPerLine))
            totalLines++;
        height = totalLines * lineHeight;
        width = lineStart + hexStart[hexPerLine] + unitWidth + asciiWidth;
        dimensionsCalculated = true;

        //System.out.println("char: " + charWidth + "x" + charHeight + ". Dimensions: " + width + "x" + height +".");
    }
    
    public void paint(Graphics g) {
        super.paint(g); // not needed
	//System.out.println("paint called. " + g.getClipBounds());
        if (dimensionsCalculated==false) {
            g.setFont(font);            
            FontMetrics fm = g.getFontMetrics();
            calcDim(fm);
        }
        
        int top, bot;
        Rectangle clipRect = g.getClipBounds();
        if (clipRect != null) {
            //If it's more efficient, draw only the area
            //specified by clipRect.
            //Top-leftmost point = (clipRect.x, clipRect.y)
            //Width, height = clipRect.width, clipRect.height
            //g.setColor(Color.white);
            //g.fillRect(clipRect.x, clipRect.y, clipRect.width, clipRect.height);
            
            top = clipRect.y / lineHeight;
            bot = (clipRect.y + clipRect.height) / lineHeight + 1;
        } else {
            top = 0;
            bot = totalLines;
            //Paint the entire component.
        }
      
        paintLines(g,top,bot);
    }

        /** paint each hex unit from start to finish */
    public void paintLines(Graphics g, int start, int finish) {
        //System.out.println("painting from " + start + "(" + start*hexPerLine + ") to " + finish + "(" + finish*hexPerLine + ")");
        
        g.setFont(font);
        FontMetrics fm = g.getFontMetrics();
        
        int len = root.getLength(); 
        byte[] data = root.getData();
        int linenum = start;
        int lastHex = finish*hexPerLine;
	int selStart = -1; 
	int selEnd = -1; 

	if (selection != null ) {
	    selStart = selection.getStart();
	    selEnd = selStart + selection.getLength();
	}

        for (int i = start*hexPerLine; i < len && i <=lastHex; i += hexPerLine) { // line (y)
            linenum++;
	    StringBuffer ascii = new StringBuffer(16);
            
            int jlen = (len-i >= hexPerLine ? hexPerLine : len-i);
            for (int j=0; j < jlen; j++) { // unit (x)
                // draw line of hex
                if (i+j >= selStart && i+j < selEnd) {
                    g.setColor(Color.magenta);
                } else {
                    g.setColor(Color.black);
                }
                g.drawString( byte2hex(data[i+j]), lineStart + hexStart[j], lineHeight*linenum);
		ascii.append(byte2ascii(data[i+j]));
            }
	    g.setColor(Color.black);
	    g.drawString(ascii.toString(), lineStart + hexStart[hexPerLine], lineHeight*linenum); //XXX: magicnumber
            
            // draw address:
            String addr = i+":  ";
            g.setColor(Color.blue);
            g.drawString( addr,lineStart - fm.stringWidth(addr),lineHeight*linenum);
        }
    }
    
    public static String byte2ascii(byte b) {
	if (b <= 20)
	    return " ";
	
	return Character.toString((char)b); //XXX: do this some other way?

    }

    public static String byte2hex(byte b) {
        int l = ((int)b & 0xf0) >> 4;
        int r = (int)b & 0x0f;

        return "" 
            + (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
            + (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
    }
}
