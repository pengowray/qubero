package net.pengo.app;

import java.awt.event.ActionEvent;

import javax.swing.AbstractAction;
import javax.swing.JCheckBoxMenuItem;
import javax.swing.JMenu;
import javax.swing.JMenuBar;
import javax.swing.JMenuItem;
import javax.swing.JSeparator;
import javax.swing.SwingUtilities;
import javax.swing.UIManager;
import javax.swing.UIManager.LookAndFeelInfo;
import javax.swing.UnsupportedLookAndFeelException;

import net.pengo.hexdraw.original.renderer.Renderer;

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
        
        nowmenu = new JMenu("View");
        /*
        ButtonGroup greyModeGroup = new ButtonGroup();
        JRadioButtonMenuItem grey0 = new JRadioButtonMenuItem( new GreyModeItem("ASCII", 0));
        JRadioButtonMenuItem grey1 = new JRadioButtonMenuItem( new GreyModeItem("Grey scale", 1));
        JRadioButtonMenuItem grey2 = new JRadioButtonMenuItem( new GreyModeItem("Grey scale II", 2));
        greyModeGroup.add(grey0);
        greyModeGroup.add(grey1);
        greyModeGroup.add(grey2);
        grey0.setSelected(true);
        nowmenu.add( grey0 );
        nowmenu.add( grey1 );
        nowmenu.add( grey2 );
         */
        
//        Renderer renderers[] = gui.getViewModes();
        //JCheckBox does not work properly in Java 1.5. Change to JCheckBoxMenuItem ?
//        JCheckBoxMenuItem checks[] = new JCheckBoxMenuItem[renderers.length];
        
//        for ( int i = 0; i < checks.length; i++) {
//            Renderer renderer = renderers[i];
//            if ( renderer == null ) {}
//            else if ( renderer.toString().equals("_") )
//                nowmenu.add( new JSeparator() );
//            else {
//                
//                ViewMenuItem action = new ViewMenuItem(renderer);
//                checks[i] = new JCheckBoxMenuItem( renderer.toString(), renderer.isEnabled() );
//                checks[i].setAction( action );
//                nowmenu.add( checks[i] );
//            }
//        }
/*
        nowmenu.add( new JSeparator() );
        JCheckBox wave = new JCheckBox( new ViewMenuItem("Wave", checks[0]) );
        JCheckBox waveRGB = new JCheckBox( new ViewMenuItem("Wave RGB", checks[0]) );
        nowmenu.add( wave );
        nowmenu.add( waveRGB );
 */
        //fixme: Should get feedback from GUI as to which grey mode is used.
        
        this.add(nowmenu);
        
        nowmenu = new JMenu("Go");
        nowmenu.add("no go.").setEnabled(false);
        this.add(nowmenu);
        
        nowmenu = new JMenu("Tools");
        nowmenu.add("no tool.").setEnabled(false);
        this.add(nowmenu);
        
        nowmenu = new JMenu("Options");
        //nowmenu.add("no option.").setEnabled(false);
        nowmenu.add( new JMenuItem(new AbstractAction("Small font size") {
            public void actionPerformed(ActionEvent e) {
                gui.fontSizeSmall();
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("Regular font size") {
            public void actionPerformed(ActionEvent e) {
                gui.fontSizeMedium();
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("Large font size") {
            public void actionPerformed(ActionEvent e) {
                gui.fontSizeLarge();
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("Extra large font size") {
            public void actionPerformed(ActionEvent e) {
                gui.fontSizeXLarge();
            }
        }));
        
        nowmenu.add( new JSeparator() );
        
        nowmenu.add( new JMenuItem(new AbstractAction("1 column") {
            public void actionPerformed(ActionEvent e) {
                gui.setColumnCount(1);
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("4 columns") {
            public void actionPerformed(ActionEvent e) {
                gui.setColumnCount(4);
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("8 columns") {
            public void actionPerformed(ActionEvent e) {
                gui.setColumnCount(8);
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("10 columns") {
            public void actionPerformed(ActionEvent e) {
                gui.setColumnCount(10);
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("16 columns") {
            public void actionPerformed(ActionEvent e) {
                gui.setColumnCount(16);
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("32 columns") {
            public void actionPerformed(ActionEvent e) {
                gui.setColumnCount(32);
            }
        }));
        nowmenu.add( new JMenuItem(new AbstractAction("100 columns") {
            public void actionPerformed(ActionEvent e) {
                gui.setColumnCount(100);
            }
        }));
        
        nowmenu.add( new JSeparator() );
        
        addLookAndFeels(nowmenu);
        
        this.add(nowmenu);
        
        nowmenu = new JMenu("Help");
        nowmenu.add("no help.").setEnabled(false);
        this.add(nowmenu);
    }
    
    private void addLookAndFeels(JMenu nowmenu) {
        
        //UIManager ui = new UIManager();
        LookAndFeelInfo[] laf = UIManager.getInstalledLookAndFeels();
        if (laf == null)
            return;
        
        //String defaultLAF = UIManager.getLookAndFeel().getName();
        
        for (LookAndFeelInfo l : laf) {
            //System.out.println("candidate look and feel name: " + l.getName());
            final LookAndFeelInfo lnf = l;
            nowmenu.add( new JMenuItem(new AbstractAction(l.getName() + " look+feel") {
                public void actionPerformed(ActionEvent action) {
                    
                    try {
                        UIManager.setLookAndFeel(lnf.getClassName());
                        SwingUtilities.updateComponentTreeUI(MoojMenuBar.this.gui.getJFrame());
                    } catch (ClassNotFoundException e) {
                        e.printStackTrace();
                    } catch (InstantiationException e) {
                        e.printStackTrace();
                    } catch (IllegalAccessException e) {
                        e.printStackTrace();
                    } catch (UnsupportedLookAndFeelException e) {
                        e.printStackTrace();
                    }
                }
            }));
            
        }
        
    }
    
    
    
    private class ViewMenuItem extends AbstractAction {
        private Renderer renderer;
        public ViewMenuItem(Renderer renderer) {
            super(renderer.toString());
            this.renderer = renderer;
        }
        
        public void actionPerformed(ActionEvent e) {
            GUI gui = MoojMenuBar.this.gui;
            
            String name = e.getActionCommand();
            Renderer renderer = gui.getViewMode(name);
            
            // read check box item value and update checks
            //JCheckBox check = (JCheckBox)(e.getSource());
            
            JCheckBoxMenuItem check = (JCheckBoxMenuItem)(e.getSource());
            renderer.setEnabled(check.isSelected());
            
        }
        
    }
}


