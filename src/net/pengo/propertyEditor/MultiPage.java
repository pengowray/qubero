/**
 * MethodSelection.java
 *
 * A way of selecting between multiple methods/classes of input for a properties page.
 * e.g. Combo box to select "Int" or "String" and pages for each type's input
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;
import java.awt.FlowLayout;

import javax.swing.BoxLayout;
import javax.swing.JPanel;



public class MultiPage extends EditablePage {
    
    private PropertyPage[] page;
    private String name;
    protected int selected = 0;
    
    private JPanel main;
    
    public MultiPage(PropertiesForm form, PropertyPage[] page, String name) {
        super();
        this.page = page;
        this.name = name;
        setForm(form);
        
        setLayout(new FlowLayout());
        
        main = new JPanel();
        
        BoxLayout layout = new BoxLayout(main, BoxLayout.Y_AXIS); //BoxLayout.Y_AXIS
        main.setLayout(layout);
        

        for (int i = 0; i < page.length; i++) {
            main.add(page[i]);
        }
        
        add(main);
        
        //main = new JPanel(layout);
        //add(main, BorderLayout.CENTER);
        
        build();
    }
    
    public void setForm(PropertiesForm form) {
        super.setForm(form);
        
        if (page == null)
            return;
        
        for (int i = 0; i < page.length; i++) {
            page[i].setForm(form);
        }
    }
    
    public String toString() {
        return name;
    }

    protected void saveOp() {
        for (int i = 0; i < page.length; i++) {
            PropertyPage p = page[i];
            p.save();
        }    
    }


    public void buildOp() {
        for (int i = 0; i < page.length; i++) {
            PropertyPage p = page[i];
            p.build();
        }
        
        // these needed?
        //revalidate();
        //repaint();
    }
    
    
    
}

