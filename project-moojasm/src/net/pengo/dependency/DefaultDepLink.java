/**
 * DefaultDepLink.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.dependency;

public class DefaultDepLink
{
    private QNode source;
    private QNode sink;
    
    /**
     * Constructor
     *
     * @param    source              a  QNode
     * @param    sink                a  QNode
     */
    public DefaultDepLink(QNode source, QNode sink) {
	this.source = source;
	this.sink = sink;
    }
    
    /**
     * Sets Source
     *
     * @param    Source              a  QNode
     */
    public void setSource(QNode source) {
	this.source = source;
    }
    
    /**
     * Returns Source
     *
     * @return    a  QNode
     */
    public QNode getSource() {
	return source;
    }
    
    /**
     * Sets Sink
     *
     * @param    Sink                a  QNode
     */
    public void setSink(QNode sink) {
	this.sink = sink;
    }
    
    /**
     * Returns Sink
     *
     * @return    a  QNode
     */
    public QNode getSink() {
	return sink;
    }
}
    

