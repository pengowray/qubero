/*
 * commandLine.java
 *
 * Created on 9 November 2004, 19:50
 */

package net.pengo.hexdraw.original;

import java.awt.BorderLayout;
import java.awt.FlowLayout;
import java.awt.event.ActionEvent;
import java.awt.event.ActionListener;
import java.io.IOException;
import javax.swing.JButton;
import javax.swing.JComboBox;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JTextField;
import net.pengo.app.ActiveFile;
import net.pengo.app.ActiveFileEvent;
import net.pengo.app.ActiveFileListener;
import net.pengo.data.ArrayData;

/**
 *
 * @author  Que
 */
public class CommandLine implements ActiveFileListener {
    
    private ActiveFile activeFile = null;
    private JPanel commandPanelHolder;
    private JTextField commandField;
    private ActionListener actionListener; // deals with submitting / pressing enter
    private long offset;
    
    /** Creates a new instance of commandLine */
    public CommandLine(ActiveFile activeFile) {
        this.activeFile = activeFile;
        activeFile.addActiveFileListener(this);
        closeCommandLine();
    }
    
    public JPanel getCommandPanel() {
        if (commandPanelHolder == null) {
            commandPanelHolder = new JPanel();
            //JPanel.setLayout()
        }
        
        return commandPanelHolder;
    }

    public void closeCommandLine() {
        JPanel cmdPanel = getCommandPanel();
        cmdPanel.setLayout(new BorderLayout());
        
        cmdPanel.removeAll();
        cmdPanel.revalidate();
        cmdPanel.repaint();
    }
    
    // show command line, read to replace text
    public void showReplace(long offset) {
        this.offset = offset;
        JPanel cmdPanel = getCommandPanel();
        cmdPanel.setLayout(new BorderLayout());
        
        cmdPanel.removeAll();
        
        //add(commandPanel, BorderLayout.SOUTH);
        
        JPanel innerCommand = new JPanel();
        JComboBox replaceInsert = new JComboBox(new String[]{"Replace", "Insert"});
        
        innerCommand.setLayout(new BorderLayout());
        
        JPanel westPanel = new JPanel();
        westPanel.setLayout(new FlowLayout(FlowLayout.LEFT,0,0));
        westPanel.add(new JLabel("addr: " + offset + " "));
        westPanel.add(replaceInsert);
                
        innerCommand.add(westPanel,BorderLayout.WEST);
        //innerCommand.add(replaceInsert,BorderLayout.WEST);
        
        JButton x = new JButton("close");
        x.addActionListener(new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                closeCommandLine();
            }
        });
        innerCommand.add(x,BorderLayout.EAST);
        commandField = new JTextField();
        // retrieve editing byte
        try {
            byte[] data = activeFile.getActive().getData().readByteArray(offset, 1);
            commandField.setText(Byte.toString(data[0]));
            innerCommand.add(commandField);
            cmdPanel.add(innerCommand);
            cmdPanel.revalidate();
            cmdPanel.repaint();
            commandField.setSelectionStart(0);
            commandField.setSelectionEnd(commandField.getText().length());
            commandField.requestFocusInWindow();
            commandField.addActionListener(getActionListener());
            
            //commandField.getDocument().
            
        } catch (IOException e) {
            //boom
            commandField.setText("IOException!");
            e.printStackTrace();
        } finally {
        }
        
    }
    
    private ActionListener getActionListener() {
        if (actionListener == null) {
            actionListener = new ActionListener() {
                public void actionPerformed(ActionEvent e) {
                    byte[] save = parse(commandField.getText());
                    
                    activeFile.getActive().getEditableData().overwrite(offset, new ArrayData(save));
                }
            };
        }
        
        return actionListener;
        
    }
    
    private byte[] parse(String s) {
        //FIXME: throws badly formatted something..
        try {
            byte b = Byte.decode(s).byteValue();
            return new byte[] { b };
        } catch (NumberFormatException e) {
            return new byte[0];
        }
    }
    
    // **** Active File Listener methods.. FIXME
    public void activeChanged(ActiveFileEvent e) {}
    public void openFileAdded(ActiveFileEvent e) {}
    public void openFileRemoved(ActiveFileEvent e) {}
    // name change on an open file
    public void openFileNameChanged(ActiveFileEvent e) {}
    public boolean readyToCloseOpenFile(ActiveFileEvent e) {return true;}
    public void closedOpenFile(ActiveFileEvent e) {}
}
