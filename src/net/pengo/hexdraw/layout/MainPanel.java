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
import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.BitSelectionEvent;
import net.pengo.bitSelection.BitSelectionListener;
import net.pengo.data.Data;

/**
 *
 * @author  Que
 */
public class MainPanel extends JPanel implements BitSelectionListener {
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
    	defaultSize = 11;
    	TextTileSet tiles = new TextTileSet("hex", false);
    	UnitSpacer unit = new UnitSpacer(tiles);
    	
    	SuperSpacer[] lineContents = new SuperSpacer[] { 
        		unit, unit, unit, unit };
    	
    	/*
    	GroupSpacer line = new GroupSpacer();
    	line.setContents(lineContents);
        line.setLength(new BitCursor(0,4)); //shouldn't need...? or should it?-
        line.setHorizontal(false);
        
        spacer = line;
        */

        RepeatSpacer column = new RepeatSpacer();
        column.setHorizontal(true);
        column.setContents(unit);
        
        spacer = column;
    	
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
    	Dimension size = new Dimension((int) spacer.getPixelWidth(len), (int) spacer.getPixelWidth(len));
    	setMinimumSize(size);
    	setMaximumSize(size);
    	setSize(size);
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
        recalc();
    }
    
    public void valueChanged(BitSelectionEvent e) {
    	recalc();
        repaint();
    }
    
    public void paintComponent(Graphics g) {
    	Data d = activeFile.getActive().getData();
    	
    	//d.
    	spacer.paint(g, activeFile.getActive().getData(), 
    			new BitSegment(new BitCursor(), d.getBitLength()) );
    }
    
}
