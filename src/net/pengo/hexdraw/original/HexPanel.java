package net.pengo.hexdraw.original;

/* this is the original hex panel. updated to work with the new selection model. */

import java.awt.Color;
import java.awt.Dimension;
import java.awt.Font;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.Rectangle;
import java.awt.datatransfer.Transferable;
import java.awt.dnd.DropTargetListener;
import java.awt.event.MouseEvent;
import java.io.IOException;
import java.io.InputStream;

import javax.swing.JPanel;
import javax.swing.Scrollable;
import javax.swing.SwingConstants;
import javax.swing.event.MouseInputAdapter;

import net.pengo.app.ActiveFile;
import net.pengo.app.ActiveFileEvent;
import net.pengo.app.ActiveFileListener;
import net.pengo.app.ClipboardEvent;
import net.pengo.app.FileEvent;
import net.pengo.app.OpenFile;
import net.pengo.app.OpenFileListener;
import net.pengo.app.SelectionEvent;
import net.pengo.data.Data;
import net.pengo.data.DataEvent;
import net.pengo.data.DataListener;
import net.pengo.data.EmptyData;
import net.pengo.hexdraw.original.renderer.AddressRenderer;
import net.pengo.hexdraw.original.renderer.AsciiRenderer;
import net.pengo.hexdraw.original.renderer.GreyScale2Renderer;
import net.pengo.hexdraw.original.renderer.GreyScaleRenderer;
import net.pengo.hexdraw.original.renderer.HexRenderer;
import net.pengo.hexdraw.original.renderer.Renderer;
import net.pengo.hexdraw.original.renderer.SeperatorRenderer;
import net.pengo.hexdraw.original.renderer.WaveRGBRenderer;
import net.pengo.hexdraw.original.renderer.WaveRenderer;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionListener;
import net.pengo.selection.LongListSelectionModel;

//FIXME: openFile's resources should be considered in rendering ?

public class HexPanel extends JPanel implements DataListener, ActiveFileListener, OpenFileListener, Scrollable, LongListSelectionListener, DropTargetListener  {
    
    private ActiveFile activeFile = null;
    protected OpenFile openFile = null; // == activeFile.getActive()
    protected Data root;
    protected long rootLength; // cached value to check if it's changed size.
    
    protected boolean dimensionsCalculated = false;
    protected long height; // height of entire "sheet" of hex
    protected int viewportHeight = 500; // preferred viewport height
    protected int width; // width of entire "sheet" of hex
    protected int lineStart; // where does the first character start
    protected int hexPerLine = 16; // number of hex units shown per line
    protected int[] hexStart; // where each hex starts on a line.
    protected int unitWidth; // length of two characters, basically.
    protected int lineHeight; // height of one line of hex
    protected long totalLines;
    protected boolean charSizeKnown = false;
    //protected int charWidth = 8, charHeight = 14; // laptop sizes
    protected int charWidth = 7, charHeight = 16, charAscent = 12; // probable sizes(?)
    
    //protected Data selection = null;
    //protected SegmentalLongListSelectionModel selectionModel = new SegmentalLongListSelectionModel();
    
    private boolean draggingMode = false; // mouse is currently dragging a selection
    //    private boolean published = false; // has the current selection been published?
    
    
    private Font font = new Font("Monospaced", Font.PLAIN, 11); // antialias?
    //private Font font = new Font("Monospaced", Font.PLAIN, 8); // antialias?


    Renderer renderers[];
    private boolean[] checks;
    
    
    public HexPanel(ActiveFile activeFile)  {
	super();
	setBackground(Color.white);

	this.activeFile = activeFile;
	activeFile.addActiveFileListener(this);
	setOpenFile(activeFile.getActive());
	
	// MOUSE ROUTINES
	
	addMouseListener(
	    new MouseInputAdapter()  {
		
		public void mousePressed(MouseEvent e)  {
		    // e.getModifiers(); // FIXME: later.
		    long hclick = hexFromClick( e.getX(), e.getY() );
		    if (hclick != -1)  {
			getSelectionModel().setValueIsAdjusting(true);
			draggingMode = true;
			
			click(hclick, e);
		    }
		    else  {
			draggingMode = false;
			getSelectionModel().setValueIsAdjusting(false);
		    }
		    
		}
		
		public synchronized void  mouseReleased(MouseEvent e)  {
		    if (draggingMode) // && selection != null
		    {
			//System.out.println("release: " + e);
			draggingMode = false;
			getSelectionModel().setValueIsAdjusting(false);
		    }
		    
		}
		
		public void mouseClicked(MouseEvent e)  {
		    int clicks = e.getClickCount();
		    if (clicks == 2) {
		    	long hclick = hexFromClick( e.getX(), e.getY() );
			if (hclick != -1)  {
			    startEdit(hclick);
			}
			
		    //getSelectionModel().setValueIsAdjusting(false);
		    //draggingMode = false;
		    }
		}
		
	    }
	);
	
	addMouseMotionListener(new MouseInputAdapter()  {
		    public void mouseDragged(MouseEvent e)  {
			long hclick = hexFromClick( e.getX(), e.getY() );
			if (draggingMode && hclick != -1)  {
			    //getSelectionModel().setLeadSelectionIndex(hclick);
			    changeSelection(hclick, false, true);
			}
			
		    }
		});
	renderers = new Renderer[] {
	        new AddressRenderer(this, true),
	        new SeperatorRenderer(this, true),
	        new HexRenderer(this, false),
	        new AsciiRenderer(this, false),
	        new SeperatorRenderer(this, true),
	        new GreyScaleRenderer(this, false),
	        new GreyScale2Renderer(this, true),
	        new SeperatorRenderer(this, true),
	        new WaveRenderer(this, false),
	        new WaveRGBRenderer(this, false),
	};
	
	// do reflection here.. later..
	//String[] classes = { "Address", "Hex", "Seperator", "Ascii"
	//renderers = new Renderer[classes.length];
	
	//renderers[0] = (Renderer)( new AddressRenderer(this, true) );
	//renderers[1] = (Renderer)( new HexRenderer(this, true) );
	//renderers[2] = (Renderer)( new SeperatorRenderer(this, true) );
	//renderers[3] = (Renderer)( new AsciiRenderer(this, true) );
	
    }
    
    public void openFileNameChanged(ActiveFileEvent e) {
	// do nothing
    }
    
    public void openFileRemoved(ActiveFileEvent e) {
	// do nothing
    }
    
    public void openFileAdded(ActiveFileEvent e) {
	// do nothing
    }
    
    public void closedOpenFile(ActiveFileEvent e) {
	// do nothing
    }
    
    public boolean readyToCloseOpenFile(ActiveFileEvent e) {
	return true;
    }
    
    public void activeChanged(ActiveFileEvent e) {
	setOpenFile(activeFile.getActive());
    }
    
    private void startEdit(long pos) {
	//Fixme: TODO
	System.out.println("now (not really) editing at: " + pos);
    }
    
    // triggered when data changes
    public void dataUpdate(DataEvent e) {
	repaint();
    }
    
    /**
     * Called whenever the value of the selection changes.
     * @param e the event that characterizes the change.
     */
    public void valueChanged(LongListSelectionEvent e)  {
	
	//LongListSelectionModel sm = openFile.getSelectionModel();
	//repaintHex(sm.getMinSelectionIndex(), sm.getMaxSelectionIndex());
	repaint();
    }

    public void click(long hclick, MouseEvent e) {
	if (e.isShiftDown())  {
	    //getSelectionModel().setLeadSelectionIndex(hclick);
	    changeSelection(hclick, false, true);
	}
	else if (e.isControlDown())  {
	    //getSelectionModel().addSelectionInterval(hclick,hclick);
	    changeSelection(hclick, true, false);
	}
	else  {
	    //getSelectionModel().setSelectionInterval(hclick,hclick);
	    changeSelection(hclick, false, false);
	}
    }
    

    public void changeSelection(long index, boolean toggle, boolean extend)  {
	//super.changeSelection(rowIndex, columnIndex, toggle, extend);
	LongListSelectionModel sm  = getSelectionModel();
	boolean selected = sm.isSelectedIndex(index);
	
	if (extend)  {
	    if (toggle)  {
		sm.setAnchorSelectionIndex(index);
	    }
	    else  {
		sm.setLeadSelectionIndex(index);
	    }
	}
	else  {
	    if (toggle)  {
		if (selected)  {
		    sm.removeSelectionInterval(index, index);
		}
		else  {
		    sm.addSelectionInterval(index, index);
		}
	    }
	    else  {
		sm.setSelectionInterval(index, index);
	    }
	}
    }
    
    
    private void setOpenFile(OpenFile openFile)  {
	if (this.openFile != null)  {
	    this.openFile.removeOpenFileListener(this);
	    this.openFile.removeLongListSelectionListener(this);
	    this.openFile.removeDataListener(this);
	}
	
	this.openFile = openFile;
	if (openFile == null) {
	    this.root = new EmptyData();
	} else {
	    this.root = openFile.getData();
	    openFile.addOpenFileListener(this);
	    //openFile.addResourceListener(this);
	    openFile.addLongListSelectionListener(this);
	    openFile.addDataListener(this);
	}
	
	//getSelectionModel().clearSelection();
	rootLength = root.getLength();
	reCalcDim();
	
	long MAX_PREC = 16777216;
	if (height > Integer.MAX_VALUE)  {
	    System.out.println("Warning: Very long file being displayed. Display height of " + height + " > Integer.MAX_VALUE (" + Integer.MAX_VALUE + ")" );
	    System.out.println("      Size is " + (float)((double)height/Integer.MAX_VALUE)*100 + "% of Integer.MAX_VALUE" );
	    System.out.println("      Size is " + (float)((double)height/MAX_PREC)*100 + "% of Max precise display height" );
	}
	else if (height > MAX_PREC )  {
	    System.out.println("Note (for old versions of Java): Longish file being displayed. Max precise display height of " + MAX_PREC + " < actual display height of " + height + " < Integer.MAX_VALUE (" + Integer.MAX_VALUE + ")" );
	    System.out.println("      Size is " + (float)((double)height/Integer.MAX_VALUE)*100 + "% of Integer.MAX_VALUE" );
	    System.out.println("      Size is " + (float)((double)height/MAX_PREC)*100 + "% of Max precise display height" );
	}
	
    }
    
    public long hexFromClick(int x, int y)  {
	
	int hx = -1;
	int hy = -1;
	
	// work out column (x coordinate)
	for (int h=0; h<hexStart.length; h++)  {
	    if (x >= lineStart+hexStart[h] && x <= lineStart+hexStart[h]+unitWidth)  {
		hx = h;
		break;
	    }
	}
	if (hx == -1)  {
	    // not found
	    return -1;
	}
	
	// work out row (y)
	hy = y / lineHeight;
	
	// work out address
	long hex = (hy * hexPerLine) + hx;
	
	// dont let selection go further than there is hex
	if (hex > rootLength)
	    hex = rootLength-1;
	
	return hex;
    }
    
    /*
     private void repaintAfterNewSelection(long newSelectionFirst, long newSelectionLast) {
     long minC, maxC; // min and max characters changed
     if (isOldSelection())
     {
     minC = Math.min(newSelectionFirst, oldSelectionFirst);
     maxC = Math.max(newSelectionLast, oldSelectionLast);
     }
     else
     {
     minC = newSelectionFirst;
     maxC = newSelectionLast;
     }
     
     repaintHex(minC, maxC);
     
     oldSelectionFirst = newSelectionFirst;
     oldSelectionLast = newSelectionLast;
     }
     */
    
    /** repaints between the two address positions.
     * e.g. repaintHex(0, 31) may repaint the first two lines
     */
    private void repaintHex(long minPos, long maxPos)  {
	//repaint only necessary areas
	//FIXME: this just gets converted back to hex units again, so why bother?
	//FIXME: doesn't trim X-ways
	
	//FIXME: loss of precision!
	repaint(0, (int)((minPos/hexPerLine)*lineHeight), width, (int)(((maxPos/hexPerLine)+1)*(lineHeight)));
	
    }
    
    /** returns a copy of the selection model */
    private LongListSelectionModel getSelectionModel()  {
	return openFile.getSelectionModel();
    }
    
    /*
     
     public synchronized void setSelection(Data sel, boolean publish)
     {
     if (sel.equals(selection) && (publish==false || published==true)) return;
     
     Data oldSel = this.selection;
     this.selection = sel;
     
     if (publish==true)
     {
     openFile.setSelection(this, selection);
     published = true;
     }
     else
     {
     published = false;
     }
     
     
     long minC, maxC; // min and max characters changed
     if (oldSel != null)
     {
     long nselStart = sel.getStart();
     long oselStart = oldSel.getStart();
     long nselEnd = nselStart + sel.getLength();
     long oselEnd = oselStart + oldSel.getLength();
     minC = Math.min(nselStart, oselStart);
     maxC = Math.max(nselEnd, oselEnd);
     }
     else
     {
     minC = sel.getStart();
     maxC = minC + sel.getLength();
     }
     
     //repaint only necessary areas
     //FIXME: this just gets converted back to hex units again, so why bother?
     //FIXME: doesn't trim X-ways
     repaint(0, (int)((minC/hexPerLine)*lineHeight), width, (int)(((maxC/hexPerLine)+1)*(lineHeight))); //FIXME: loss of precision!
     }
     
     public void setSelection(long offset, long len)
     {
     setSelection(offset, len, true);
     }
     public void setSelection(long offset, long len, boolean publish)
     {
     if (offset >= root.getLength())
     {
     // off the board!
     clearSelection();
     return;
     }
     if (offset + len > root.getLength())
     {
     len = root.getLength() - offset;
     }
     setSelection(root.getSelection(offset, len), publish);
     }
     */
    
    public Dimension getPreferredSize()  {
        //FIXME: this is a hack!
        return new Dimension(width, (int)height); //FIXME: precision loss!
        
        
    }
    
    public Dimension getMinimumSize()  {
        return getPreferredSize();
    }
    
    
    public Dimension getMaximumSize()  {
        return new Dimension(width, (int)height); //FIXME: precision loss!
    }
    
    //FIXME: will need def listener?
    
    private void reCalcDim()  {
        dimensionsCalculated = false;
        calcDim();
    }
    
    /** calculate dimensions */
    private void calcDim()  {
        if (dimensionsCalculated == true)
            return;
        
        //calcDim(getComponentGraphics().getFontMetrics(font)); // oops stupid no work
        
        calcDim(charWidth, charHeight); //FIXME: may not have been worked out from FontMetrics
        if (charSizeKnown == false)  {
            // don't trust size until calculated with FontMetrics
            dimensionsCalculated = false;
        }
    }
    
    private void calcDim(FontMetrics fm)  {
        if (dimensionsCalculated == true)
            return;
        
        if (charSizeKnown == false)  {
            int oldCharWidth = charWidth;
            int oldCharHeight = charHeight;
            int oldCharAscent = charAscent;
            charWidth = fm.charWidth('W');
            charHeight = fm.getHeight();
            charAscent = fm.getAscent();
            charSizeKnown = true;
            if (oldCharWidth != charWidth || oldCharHeight != charHeight)  {
                System.out.println("Note: screen character size different to expected: " + charWidth  + "x" + charHeight + ",ascent:" + charAscent +  " instead of expected: " + oldCharWidth  + "x" + oldCharHeight + ",ascent:" + oldCharAscent);
            }
            if (oldCharAscent != charAscent )  {
                System.out.println("Note: character ascent different to expected: " + charAscent + " instead of expected: " + oldCharAscent);
            }
        }
        
        calcDim(charWidth, charHeight);
    }
    
    protected synchronized void calcDim(int charWidth, int charHeight)  {
        if (dimensionsCalculated == true)
            return;
        
        long oldHeight = height;
        int oldWidth = width;
        int oldLineStart = lineStart;
        
        lineHeight = charHeight;
        lineStart = charWidth * ((root.getLength()+"").length() + 3); // longest "hex" address can be (as a string), plus three (for a " : ")
        
        unitWidth = charWidth * 2; // length of two characters (eg FF).
        int asciiWidth = charWidth * hexPerLine; // how long the ascii stuff is
        
        int[] spaceEvery = new int[] {1, 2, 4, 8};
        int[] spaceSize = new int[]  {charWidth*3, 0, charWidth, charWidth };
        
        hexStart = new int[hexPerLine+1]; // where each hex starts on a line + where ascii starts
        
        int s = 0;
        for (int n=0; n<hexStart.length; n++) { // only place hexStart.length should be used. use hexPerLine instead.
            hexStart[n] = s;
            for (int m=0; m<spaceEvery.length; m++)  {
                if ((n+1) % spaceEvery[m] == 0)  {
                    s += spaceSize[m];
                }
            }
        }
        
        // if (root.isFixedLength()) {  // --- how to deal with non fixed length?
        
        totalLines = root.getLength() / hexPerLine;
        if ((float)totalLines != ((float)root.getLength() / hexPerLine))
            totalLines++;
        height = totalLines * lineHeight;
        //FIXME!!!!!!!!!
        width = lineStart + hexStart[hexPerLine] + unitWidth + asciiWidth;
        width += 500;
        dimensionsCalculated = true;
        
        if (oldWidth != width || oldHeight != height || oldLineStart != lineStart)  {
            revalidate();
        }
        //System.out.println("char: " + charWidth + "x" + charHeight + ". Dimensions: " + width + "x" + height +".");
    }
    
    public void paintComponent(Graphics g)  {
        if (rootLength != root.getLength()) { // FIXME: size change should be given via trigger
            reCalcDim();
        }
        Graphics2D g2 = (Graphics2D)g;
        super.paintComponent(g2);
        //System.out.println("paint called., width:"+width + ", height:"+height);
        //System.out.println("paint called. " + g.getClipBounds());
        //System.out.println("this dimens.. " + this.getSize());
        
        if (dimensionsCalculated==false)  {
            g2.setFont(font);
            FontMetrics fm = g2.getFontMetrics();
            calcDim(fm);
        }
        int top, bot;
        Rectangle clipRect = g2.getClipBounds();
        if (clipRect != null)  {
            //If it's more efficient, draw only the area specified by clipRect.
            
	    /*
	     // box around the just painted area
	     g2.setColor(Color.pink);
	     g2.drawRect(clipRect.x, clipRect.y, clipRect.x+clipRect.width-1, clipRect.y+clipRect.height-1);
	     g2.setColor(Color.red);
	     g2.drawLine(clipRect.x, clipRect.y, clipRect.x+clipRect.width-1, clipRect.y); // top
	     g2.setColor(Color.green);
	     g2.drawLine(clipRect.x, clipRect.y+clipRect.height-1, clipRect.x+clipRect.width-1, clipRect.y+clipRect.height-1); // bot
	     */
            
            top = clipRect.y / lineHeight;
            bot = (clipRect.y + clipRect.height) / lineHeight + 1;
            
        }
        else  {
            top = 0;
            bot = (int)totalLines; //FIXME: precision loss!!
            //Paint the entire component.
            System.out.println("Warning: Whole hexpanel component got painted! that shouldn't really happen.");
        }
        
        paintLines(g2,top,bot);
    }
    
    /** paint each hex unit from start to finish (lines) */
    public void paintLines(Graphics g, long start, long finish)  {
        //System.out.println("painting from " + start + "(" + start*hexPerLine + ") to " + finish + "(" + finish*hexPerLine + ")");
        int charsHeight = lineHeight;
        //System.out.println("rendering: "+start +"-"+finish);
        //g = g.create(0,0,width,charsHeight);
        g.setFont(font);
        
        FontMetrics fm = g.getFontMetrics();

        if (start < 0)  {
            //fixme
            System.out.println("drawing from negative! (" + start +")");
            start = 0;
        }
        
        long startByte = start*hexPerLine;
        long endByte = finish*hexPerLine;
        
        long len = root.getLength();
        if (endByte > root.getLength())
            endByte = root.getLength();
        
        long linenum = start;
        long lastHex = finish*hexPerLine;
        //long selStart = -1;
        //long selEnd = -1;
        

        boolean selecta[] = new boolean[hexPerLine]; // current selecteds
        byte ba[] = new byte[hexPerLine]; // current bytes
        
		int totalRenderWidth, segmentWidth; 
		long i;
		int index, count;
	    Renderer renderer;
	    
	    //g.setClip( 0, (int)(start)*fm.getHeight(), width, (int)(finish)*fm.getHeight()+fm.getHeight() ); // possible problems with large i[ndex]
	    g.translate( 0,(int)(start)*fm.getHeight() );
	    
        try  {
            InputStream data = root.getDataStream(startByte,endByte-startByte);
            
            for (linenum = start*hexPerLine; linenum < len && linenum <=lastHex; linenum+=hexPerLine) { // line (y)
                // calc characters to draw on this line (jlen). Can't be moreo than hexPerLine.
                count = (int)((len-linenum >= hexPerLine ? hexPerLine : len-linenum));
				totalRenderWidth = 0;
 	

                try  {
                  //count = data.read(ba, 0, count); // read bytes into ba[]
				  data.read(ba); // read bytes into ba[]
				  //count = data.read(ba); // read bytes into ba[]
                }
                catch (IOException e)  {
                    //FIXME:
                    e.printStackTrace();
                    return;
                }
                    
                // draw line of hex
                /*
                boolean selected =
                    openFile.getSelectionModel().isSelectedIndex(i+j) ||
                    (i+j >= activeSelectionFirst && i+j <= activeSelectionLast);
                */
                
                //renderByte( Point p1, Point p2, int index, byte b, boolena selected);
                
                
				//System.out.println("i:"+linenum +", lastHex:"+lastHex +", len:"+len +", count:"+count);
				//g.setClip( 0, (int)(linenum), width, charsHeight ); // possible problems with large i[ndex]

				for ( index=0; index < selecta.length; index++) {	//setup selection
                	selecta[index] = openFile.getSelectionModel().isSelectedIndex(((int)linenum *charsHeight) +index);
				}
				
				for ( index=0; index < renderers.length; index++) {	//render each renderer
                    renderer = renderers[index];
					if (renderer != null && renderer.isEnabled()) {
						segmentWidth = 
						    renderer.renderBytes( g, hexStart, linenum, ba, selecta, count );
						totalRenderWidth += segmentWidth;
		                g.translate( segmentWidth, 0 );
					}
				}
				//System.out.println("totalRenderWidth: " + totalRenderWidth +", charsHeight:" + charsHeight);
                g.translate( -totalRenderWidth, charsHeight );
                //linenum++;
            }
        } catch (IOException e)  {
            //FIXME: now what?
			e.printStackTrace();
        }
    }
    

    
    public void dataEdited(DataEvent e)  {
    }
    
    public void dataLengthChanged(DataEvent e)  {
    }
    
    public void fileSaved(FileEvent e)  {
    }
    
    public void fileClosed(FileEvent e)  {
	setOpenFile(new OpenFile(new EmptyData()));
    }
    public void selectionCopied(ClipboardEvent e)  {
    }
    
    public void selectionMade(SelectionEvent e)  {
	if (e.getSource() == this)  {
	    return;
	}
	
	//setSelection(e.getSelectionResource(), true); //FIXME:
	repaint(); //FIXME: smaller area?
	
    }
    
    public void selectionCleared(SelectionEvent e)  {
	
    }
    public Renderer[] getViewModes() {
        return renderers;
    }
    
    public Renderer getViewMode(String name)  {
        for ( int i = 0; i < renderers.length; i++) {
            if (renderers[i] != null && renderers[i].toString().equals(name) )
                return renderers[i];
        }
        return null;
    }

/*
 	public void repaint()  {
		super.repaint();
	}
*/

    
    public java.awt.Dimension getPreferredScrollableViewportSize()  {
	return new Dimension(width, viewportHeight);
    }
    
    public int getScrollableBlockIncrement(java.awt.Rectangle visibleRect, int orientation, int direction)  {
	if (orientation == SwingConstants.VERTICAL)  {
	    return visibleRect.height - lineHeight;
	}
	else  {
	    return visibleRect.width - unitWidth;
	}
    }
    
    public boolean getScrollableTracksViewportHeight()  {
	return false;
    }
    
    public boolean getScrollableTracksViewportWidth()  {
	return false;
    }
    
    public int getScrollableUnitIncrement(java.awt.Rectangle visibleRect, int orientation, int direction)  {
	if (orientation == SwingConstants.VERTICAL)  {
	    return lineHeight;
	}
	else  {
	    return unitWidth;
	}
    }
    
    public void dragEnter(java.awt.dnd.DropTargetDragEvent dropTargetDragEvent)  {
    }
    
    public void dragExit(java.awt.dnd.DropTargetEvent dropTargetEvent)  {
    }
    
    public void dragOver(java.awt.dnd.DropTargetDragEvent dropTargetDragEvent)  {
    }
    
    public void drop(java.awt.dnd.DropTargetDropEvent dtde)  {
	dtde.acceptDrop(dtde.getDropAction());
	System.out.println("accepted: " + dtde);
	
	Transferable t = dtde.getTransferable();
	System.out.println("transferable: " + t);
	System.out.println("local: " + dtde.isLocalTransfer());
	
	dtde.dropComplete(false);
    }
    
    public void dropActionChanged(java.awt.dnd.DropTargetDragEvent dropTargetDragEvent)  {
    }
    
}
