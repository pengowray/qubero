package net.pengo.app;

import java.awt.event.ActionEvent;

import javax.swing.AbstractAction;
import javax.swing.ButtonGroup;
import javax.swing.JMenu;
import javax.swing.JMenuBar;
import javax.swing.JMenuItem;
import javax.swing.JRadioButtonMenuItem;
import javax.swing.JSeparator;

public class MoojMenuBar extends JMenuBar {
    protected final GUI gui;
    public MoojMenuBar(GUI gui) {
        super();
        this.gui = gui;
        setup();
    }
    
    protected void setup() {
        JMenu nowmenu; // temp menu holder
        nowmenu = new JMenu("File");
        nowmenu.add( new JMenuItem(new AbstractAction("Close") {
            public void actionPerformed(ActionEvent e) {
                gui.closeActive();
                //gui.closeAll();
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("Close all") {
            public void actionPerformed(ActionEvent e) {
                gui.closeAll();
            }
        }));
        
        nowmenu.add( new OpenAction(gui) );
        nowmenu.add( new OpenClassAction(gui) );
        
        nowmenu.add( new JMenuItem("Save")).setEnabled(false);
        nowmenu.add( new JMenuItem(new AbstractAction("Save as...") {
            public void actionPerformed(ActionEvent e) {
                gui.saveAs();
            }
        }));
        nowmenu.add( new JMenuItem("Print...")).setEnabled(false);
        nowmenu.add( new JSeparator() );
        nowmenu.add( new JMenuItem(new ExitAction(gui))); // "Exit"
        this.add(nowmenu);
        
        nowmenu = new JMenu("Edit");
        nowmenu.add("no undo.").setEnabled(false);
        this.add(nowmenu);
        
        ButtonGroup greyModeGroup = new ButtonGroup();
        JRadioButtonMenuItem grey0 = new JRadioButtonMenuItem( new GreyModeItem("ASCII", 0));
        JRadioButtonMenuItem grey1 = new JRadioButtonMenuItem( new GreyModeItem("Grey scale", 1));
        JRadioButtonMenuItem grey2 = new JRadioButtonMenuItem( new GreyModeItem("Grey scale II", 2));
        greyModeGroup.add(grey0);
        greyModeGroup.add(grey1);
        greyModeGroup.add(grey2);
        grey0.setSelected(true); //fixme: should check.
        //fixme: Should get feedback from GUI as to which grey mode is used.
        
        nowmenu = new JMenu("View");
        
        nowmenu.add( grey0 );
        nowmenu.add( grey1 );
        nowmenu.add( grey2 );
        this.add(nowmenu);
        
        nowmenu = new JMenu("Go");
        nowmenu.add("no go.").setEnabled(false);
        this.add(nowmenu);
        
        nowmenu = new JMenu("Tools");
        nowmenu.add("no tool.").setEnabled(false);
        this.add(nowmenu);
        
        nowmenu = new JMenu("Options");
        nowmenu.add("no option.").setEnabled(false);
        this.add(nowmenu);
        
        nowmenu = new JMenu("Help");
        nowmenu.add("no help.").setEnabled(false);
        this.add(nowmenu);
    }
    
    private class GreyModeItem extends AbstractAction {
        int value;
        public GreyModeItem(String name, int value) {
            super(name);
            this.value = value;
        }
        public void actionPerformed(ActionEvent e) {
            GUI gui = MoojMenuBar.this.gui;
            gui.setGreyMode(value);
        }
        
    }
}


