/*
 * MainPanel.java
 *
 * Will layout the Spacers. Allow adding/removing of spacers, Organise them into pages or sections. Split display, 
 * export commands for GUI to show in menu.
 *
 * Created on 25 November 2004, 01:27
 */

package net.pengo.hexdraw.layout;

import java.awt.Dimension;
import java.awt.Graphics;
import javax.swing.JPanel;

import net.pengo.app.ActiveFile;
import net.pengo.app.ActiveFileEvent;
import net.pengo.app.ActiveFileListener;
import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.BitSelectionEvent;
import net.pengo.bitSelection.BitSelectionListener;
import net.pengo.data.Data;

/**
 *
 * @author  Que
 */
public class MainPanel extends JPanel implements BitSelectionListener, ActiveFileListener {
    /**
	 * Comment for <code>serialVersionUID</code>
	 */
	private static final long serialVersionUID = 1L;
	
	//private List<Spacer> spacerlist = new LinkedList<Spacer>();
	private SuperSpacer spacer;
    private int defaultSize = 11;
    private ActiveFile activeFile;
    
    /** Creates a new instance of MainPanel */
    public MainPanel(ActiveFile activeFile) {
    	loadDefaults();
        setActiveFile(activeFile);
        
    }

    public void loadDefaults() {
    	defaultSize = 12;
    	TextTileSet tiles = new TextTileSet("hex", false);
    	UnitSpacer unit = new UnitSpacer(tiles);
    	
    	AsciiTileSet asciiTiles = new AsciiTileSet("hex", false);
    	UnitSpacer asciiUnit = new UnitSpacer(asciiTiles);

        RepeatSpacer row = new RepeatSpacer();
        row.setHorizontal(true);
        row.setContents(unit);
        row.setMaxRepeats(32);
        
        RepeatSpacer asciiRow = new RepeatSpacer();
        asciiRow.setHorizontal(true);
        asciiRow.setContents(asciiUnit);
        asciiRow.setMaxRepeats(16);
        
        GroupSpacer rowWrap = new GroupSpacer();
    	rowWrap.setContents(new SuperSpacer[] { row } );
    	rowWrap.setLength(new BitCursor(16,0));
    	//rowWrap.setHorizontal(false); // n/a

        RepeatSpacer column = new RepeatSpacer();
        column.setHorizontal(false);
        //column.setContents(rowWrap);
        column.setContents(row);
        
        RepeatSpacer asciiColumn = new RepeatSpacer();
        asciiColumn.setHorizontal(false);
        asciiColumn.setContents(asciiRow);

    	SuperSpacer[] pageContents = new SuperSpacer[] { 
        		column, asciiColumn };

    	GroupSpacer page = new GroupSpacer();
    	page.setContents(pageContents);
    	//page.setContents(pageContents);
    	//page.setLength(new BitCursor(0,4)); //shouldn't need...? or should it?-
    	page.setHorizontal(true);
        
       spacer = page;
        
       //spacer = column;
        
        //spacer = row;
        
    	recalc();
    }
    public void loadDefaults_proper() {
        defaultSize = 11;
        
        //spacerlist.clear();
        //MonoSpacer hex = new MonoSpacer();
        //hex.setFontName("hex");
        //hex.setActiveFile(getActiveFile());
        //spacerlist.add(hex);
        
        TextTileSet tiles = new TextTileSet("hex", false);
        
        // this needs to be replaced with a limited repeat.. cause the same unit will be shown every time
        UnitSpacer unit = new UnitSpacer(tiles);
        
        SuperSpacer[] lineContents = new SuperSpacer[] { 
        		unit, unit, unit, unit,  unit, unit, unit, unit, 
				unit, unit, unit, unit,  unit, unit, unit, unit };
        
        GroupSpacer line = new GroupSpacer();
        line.setContents(lineContents);
        line.setLength(new BitCursor(0,4)); //shouldn't need...? or should it?-
        line.setHorizontal(true);
        
        RepeatSpacer column = new RepeatSpacer();
        column.setHorizontal(false);
        column.setContents(line);
        
        spacer = column;
        
        recalc();
        
        repack();
    }
    
    /* fix up size */
    protected void recalc() {
    	if (activeFile == null)
    		return;
    	
    	BitCursor len = activeFile.getActive().getData().getBitLength();
    	Dimension size = new Dimension((int) spacer.getPixelWidth(len), (int) spacer.getPixelHeight(len));
    	setMinimumSize(size);
    	setMaximumSize(size);
    	setPreferredSize(size);
    	setSize(size);
    	System.out.println("size:" + size);
    	//doesn't help
    	invalidate();
    	repaint();
    }
    
    
    
    /* work out how to lay everything out again.. after changes to layout properties*/
    private void repack() {
    	
    	
    	/*
        FlowLayout layout = new FlowLayout();
        layout.setHgap(10);
        layout.setVgap(0);
        layout.setAlignment(FlowLayout.LEFT);
        setLayout(layout);
        removeAll();
        for (Spacer s : spacerlist) {
            add(s);
        }
        */
        
    }

    public ActiveFile getActiveFile() {
        return activeFile;
    }

    public void setActiveFile(ActiveFile activeFile) {
        this.activeFile = activeFile;
        activeFile.getActive().getSelection().addBitSelectionListener(this);
        activeFile.addActiveFileListener(this);
        recalc();
    }
    
    public void valueChanged(BitSelectionEvent e) {
    	recalc();
        repaint();
    }
    
    public void paintComponent(Graphics g) {

    	super.paintComponent(g);

    	Data d = activeFile.getActive().getData();
    	
    	System.out.println("d.getBitLength():" + d.getBitLength());
    	spacer.paint(g, activeFile.getActive().getData(), 
    			new BitSegment(new BitCursor(), d.getBitLength()) );
    	
    }

	public void activeChanged(ActiveFileEvent e) {
		recalc();
		
	}

	public void openFileAdded(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		
	}

	public void openFileRemoved(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		
	}

	public void openFileNameChanged(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		
	}

	public boolean readyToCloseOpenFile(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		return true;
	}

	public void closedOpenFile(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		
	}
    
}
