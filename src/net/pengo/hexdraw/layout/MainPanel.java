/*
 * MainPanel.java
 *
 * Will layout the Spacers. Allow adding/removing of spacers, Organise them into pages or sections. Split display, 
 * export commands for GUI to show in menu.
 *
 * Created on 25 November 2004, 01:27
 */

package net.pengo.hexdraw.layout;

import java.awt.FlowLayout;
import java.util.LinkedList;
import java.util.List;
import javax.swing.JPanel;
import net.pengo.app.ActiveFile;
import net.pengo.bitSelection.BitSelectionEvent;
import net.pengo.bitSelection.BitSelectionListener;

/**
 *
 * @author  Que
 */
public class MainPanel extends JPanel implements BitSelectionListener {
    private List<Spacer> spacerlist = new LinkedList<Spacer>();
    private int defaultSize = 11;
    private ActiveFile activeFile;
    
    /** Creates a new instance of MainPanel */
    public MainPanel() {
        
    }
    
    public void loadDefaults() {
        defaultSize = 11;
        spacerlist.clear();
        MonoSpacer hex = new MonoSpacer();
        hex.setFontName("hex");
        hex.setActiveFile(getActiveFile());
        spacerlist.add(hex);
        
        repack();
    }
    
    
    
    /* work out how to lay everything out again.. after changes to layout properties*/
    private void repack() {
        FlowLayout layout = new FlowLayout();
        layout.setHgap(10);
        layout.setVgap(0);
        layout.setAlignment(FlowLayout.LEFT);
        setLayout(layout);
        removeAll();
        for (Spacer s : spacerlist) {
            add(s);
        }
        
    }

    public ActiveFile getActiveFile() {
        return activeFile;
    }

    public void setActiveFile(ActiveFile activeFile) {
        this.activeFile = activeFile;
        activeFile.getActive().getSelection().addBitSelectionListener(this);
    }
    
    public void valueChanged(BitSelectionEvent e) {
        repaint();
    }
    
}
